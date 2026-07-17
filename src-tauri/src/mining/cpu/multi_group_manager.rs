// Copyright 2024. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! Multi-group CPU mining manager for Windows systems with >64 logical processors.
//!
//! On Windows, each processor group caps at 64 CPUs. When the system has more than
//! 64 threads, xmrig must be spawned once per group so that every thread can use
//! huge pages (SetProcessGroupAffinity). This module handles spawning one process-wrapper
//! instance per group, monitoring all of them, and aggregating their status into a
//! single CpuMinerStatus broadcast.

use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::Mutex;
use std::time::Duration;

use log::{info, warn};
use serde::Deserialize;
use tari_shutdown::Shutdown;
use tokio::sync::watch::Sender;
use uuid::Uuid;

use crate::LOG_TARGET_APP_LOGIC;
use crate::mining::cpu::CpuMinerStatus;
use crate::port_allocator::PortAllocator;

/// Number of groups on the system (Windows only, when >64 threads).
pub fn num_groups() -> u16 {
    #[cfg(windows)]
    {
        unsafe {
            windows_sys::Win32::System::Threading::GetMaximumProcessorGroupCount()
        }
    }
    #[cfg(not(windows))]
    0
}

/// Returns the number of active processors in a specific group.
#[cfg(windows)]
pub fn get_group_processor_count(group: u16) -> u32 {
    unsafe {
        windows_sys::Win32::System::Threading::GetActiveProcessorCount(group)
    }
}

/// Returns the number of active processors in a specific group (non-Windows fallback).
#[cfg(not(windows))]
pub fn get_group_processor_count(_group: u16) -> u32 {
    0
}

// ─── Per-instance state ───────────────────────────────────────────────

struct XmrigInstance {
    http_port: u16,
    http_token: String,
    shutdown: Shutdown,
    /// PID of the process-wrapper child (used for active termination).
    #[cfg(windows)]
    wrapper_pid: Option<u32>,
}

/// Manages one xmrig process per processor group.
pub struct MultiGroupXmrigManager {
    instances: Vec<XmrigInstance>,
    summary_broadcast: Sender<CpuMinerStatus>,
    /// Shared flag — when triggered all instances shut down.
    shutdown_signal: Mutex<Shutdown>,
}

impl MultiGroupXmrigManager {
    pub fn new(summary_broadcast: Sender<CpuMinerStatus>) -> Self {
        let num_groups = num_groups();
        info!(target: LOG_TARGET_APP_LOGIC, "Multi-group mining detected {} processor groups", num_groups);

        let mut instances = Vec::with_capacity(num_groups as usize);
        for group in 0..num_groups {
            let port = PortAllocator::new().assign_port_with_fallback();
            let token = Uuid::new_v4().to_string();
            info!(target: LOG_TARGET_APP_LOGIC, "Group {}: HTTP port={}", group, port);
            instances.push(XmrigInstance {
                http_port: port,
                http_token: token,
                shutdown: Shutdown::new(),
                #[cfg(windows)]
                wrapper_pid: None,
            });
        }

        Self {
            instances,
            summary_broadcast,
            shutdown_signal: Mutex::new(Shutdown::new()),
        }
    }

    // ─── Spawning ──────────────────────────────────────────────────────

    /// Spawn one xmrig process per group. Returns the binary path on success so the caller can
    /// verify it exists before starting health checks.
    pub fn spawn(
        &mut self,
        _base_path: PathBuf,
        log_dir: PathBuf,
        binary_version_path: PathBuf,
        cpu_threads_per_group: u32,
        connection_type_args: Vec<String>,
        extra_options: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        let xmrig_binary = binary_version_path.clone();

        // Pre-spawn check: verify the binary exists before attempting to spawn.
        // This provides a clear, actionable error message instead of a generic OS-level failure.
        if !xmrig_binary.exists() {
            return Err(anyhow::anyhow!(
                "Xmrig binary not found at {}. \n\nThis usually means antivirus software has quarantined the file. \nTo fix:\n1. Open your antivirus or Windows Defender security center\n2. Check 'Quarantine' or 'Protection history'\n3. Restore '{}' and add both '{}' and its directory to your antivirus exclusions",
                xmrig_binary.display(),
                "xmrig.exe",
                "xmrig.exe"
            ));
        }

        for (group_idx, instance) in self.instances.iter_mut().enumerate() {
            // Build xmrig arguments
            let mut args = connection_type_args.clone();

            // Log file per group
            let log_file = log_dir.join("xmrig").join(format!("xmrig_group{}.log", group_idx));
            if let Some(log_path) = log_file.to_str() {
                args.push(format!("--log-file={}", log_path));
            }

            std::fs::create_dir_all(
                log_file.parent().expect("Could not get xmrig root log dir"),
            )?;

            // HTTP API
            args.push(format!("--http-port={}", instance.http_port));
            args.push(format!("--http-access-token={}", instance.http_token));
            args.push("--donate-level=1".to_string());

            // Threads for this group
            if cpu_threads_per_group > 0 {
                args.push(format!("--threads={cpu_threads_per_group}"));
            }

            args.push("--verbose".to_string());
            for opt in &extra_options {
                args.push(opt.clone());
            }

            // Build process-wrapper command:
            //   process-wrapper --group <idx> <parent_pid> <xmrig_binary> [args...]
            let wrapper_path = crate::process_wrapper::get_wrapper_path()
                .ok_or_else(|| anyhow::anyhow!("Process wrapper not initialized"))?;
            let parent_pid = std::process::id();

            let mut cmd_args: Vec<String> = vec![
                "--group".to_string(),
                group_idx.to_string(),
                parent_pid.to_string(),
                xmrig_binary.to_str().unwrap_or_default().to_string(),
            ];
            cmd_args.extend(args);

            info!(target: LOG_TARGET_APP_LOGIC, "Spawning xmrig on group {} (port {}) with args: {:?}", group_idx, instance.http_port, cmd_args);

            let child = std::process::Command::new(wrapper_path)
                .args(&cmd_args)
                .creation_flags({
                    use crate::consts::PROCESS_CREATION_NO_WINDOW;
                    PROCESS_CREATION_NO_WINDOW
                })
                .spawn()
                .map_err(|e| {
                    anyhow::anyhow!("Failed to spawn xmrig on group {}: {}", group_idx, e)
                })?;

            let wrapper_pid = child.id();
            info!(target: LOG_TARGET_APP_LOGIC, "xmrig spawned on group {} with PID {}", group_idx, wrapper_pid);

            // Store PID for active termination in stop() and is_running()
            #[cfg(windows)]
            { instance.wrapper_pid = Some(wrapper_pid); }

            // Keep child handle for cleanup (we need it to wait on exit)
            std::mem::forget(child);
        }

        Ok(())
    }

    /// Aggregate status from all instances and broadcast.
    pub async fn aggregate_status(&self) -> CpuMinerStatus {
        let mut total_hash_rate: f64 = 0.0;
        let mut is_mining = false;
        let mut is_connected = false;

        for (group_idx, instance) in self.instances.iter().enumerate() {
            match Self::fetch_status(
                instance.http_port,
                &instance.http_token,
            ).await {
                Ok(status) => {
                    total_hash_rate += status.hash_rate;
                    if status.is_mining {
                        is_mining = true;
                    }
                    if status.connection.is_connected {
                        is_connected = true;
                    }
                }
                Err(e) => {
                    warn!(target: LOG_TARGET_APP_LOGIC, "Failed to fetch status from group {}: {}", group_idx, e);
                }
            }
        }

        let aggregated = CpuMinerStatus {
            is_mining,
            estimated_earnings: 0,
            hash_rate: total_hash_rate,
            connection: crate::mining::cpu::CpuMinerConnectionStatus {
                is_connected,
            },
        };

        let _ = self.summary_broadcast.send(aggregated.clone());
        aggregated
    }

    /// Aggregate status from port/token pairs without needing &self.
    /// Used by callers who extract ports while holding a lock guard, then drop it before awaiting
    /// to avoid non-Send guards across await points in Tauri command handlers.
    pub async fn aggregate_from_ports(ports: &[(u16, String)]) -> CpuMinerStatus {
        let mut total_hash_rate: f64 = 0.0;
        let mut is_mining = false;
        let mut is_connected = false;

        for (group_idx, (http_port, http_token)) in ports.iter().enumerate() {
            match Self::fetch_status(*http_port, http_token).await {
                Ok(status) => {
                    total_hash_rate += status.hash_rate;
                    if status.is_mining {
                        is_mining = true;
                    }
                    if status.connection.is_connected {
                        is_connected = true;
                    }
                }
                Err(e) => {
                    warn!(target: LOG_TARGET_APP_LOGIC, "Failed to fetch status from group {}: {}", group_idx, e);
                }
            }
        }

        CpuMinerStatus {
            is_mining,
            estimated_earnings: 0,
            hash_rate: total_hash_rate,
            connection: crate::mining::cpu::CpuMinerConnectionStatus {
                is_connected,
            },
        }
    }

    /// Poll a single xmrig instance's HTTP API.
    async fn fetch_status(http_port: u16, token: &str) -> Result<CpuMinerStatus, anyhow::Error> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/2/summary", http_port))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        let text = response.text().await?;
        #[derive(Deserialize)]
        struct Summary {
            connection: Connection,
            hashrate: Hashrate,
        }
        #[derive(Deserialize)]
        struct Connection {
            uptime: u64,
        }
        #[derive(Deserialize)]
        struct Hashrate {
            total: Vec<Option<f64>>,
        }

        let body: Summary = serde_json::from_str(&text)?;

        let (ten_s, sixty_s, fifteen_m) = (
            body.hashrate.total.first().and_then(|v| *v),
            body.hashrate.total.get(1).and_then(|v| *v),
            body.hashrate.total.get(2).and_then(|v| *v),
        );

        let avg_hash_rate = fifteen_m.or(sixty_s).or(ten_s).unwrap_or(0.0);

        Ok(CpuMinerStatus {
            is_mining: true,
            estimated_earnings: 0,
            hash_rate: avg_hash_rate,
            connection: crate::mining::cpu::CpuMinerConnectionStatus {
                is_connected: body.connection.uptime > 0,
            },
        })
    }

    /// Trigger shutdown on all instances and actively terminate xmrig processes.
    pub async fn stop(&self) -> Result<(), anyhow::Error> {
        info!(target: LOG_TARGET_APP_LOGIC, "Stopping multi-group xmrig ({} groups)", self.instances.len());

        // Signal the status aggregation loop to stop polling
        if let Ok(mut signal) = self.shutdown_signal.lock() {
            signal.trigger();
            *signal = Shutdown::new();
        }

        // Extract PIDs before dropping any borrow of self — this avoids holding
        // a non-Send RwLockWriteGuard across an await point in callers.
        let pids: Vec<u32> = self.instances.iter()
            .filter_map(|inst| inst.wrapper_pid)
            .collect();

        Self::terminate_all(&pids).await;

        Ok(())
    }

    /// Terminate all wrapper processes by PID without needing &self.
    /// Used by callers who extract PIDs from the manager before releasing a lock guard,
    /// avoiding non-Send guards across await points in Tauri command handlers.
    pub async fn terminate_all(pids: &[u32]) {
        if pids.is_empty() { return; }

        info!(target: LOG_TARGET_APP_LOGIC, "Terminating {} wrapper processes", pids.len());

        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
            use windows_sys::Win32::System::Threading::{
                GetExitCodeProcess, OpenProcess, TerminateProcess,
            };

            for (group_idx, &wrapper_pid) in pids.iter().enumerate() {
                info!(target: LOG_TARGET_APP_LOGIC, "Terminating group {} wrapper PID {}", group_idx, wrapper_pid);

                unsafe {
                    // Open the process with terminate access
                    const SYNCHRONIZE: u32 = 0x0010_0000;
                    const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
                    let desired_access = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;
                    let handle = OpenProcess(desired_access, 0, wrapper_pid);

                    if handle != 0 {
                        // First try graceful termination via taskkill (tree kill)
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/T", "/PID", &wrapper_pid.to_string()])
                            .creation_flags({
                                use crate::consts::PROCESS_CREATION_NO_WINDOW;
                                PROCESS_CREATION_NO_WINDOW
                            })
                            .output();

                        // Wait briefly for graceful shutdown
                        let start = std::time::Instant::now();
                        while start.elapsed() < Duration::from_secs(5) {
                            let mut exit_code: u32 = 0;
                            if GetExitCodeProcess(handle, &mut exit_code) != 0
                                && exit_code != STILL_ACTIVE as u32
                            {
                                info!(target: LOG_TARGET_APP_LOGIC, "Group {} wrapper exited gracefully", group_idx);
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(200));
                        }

                        // Force terminate if still running
                        let mut exit_code: u32 = 0;
                        if GetExitCodeProcess(handle, &mut exit_code) != 0
                            && exit_code == STILL_ACTIVE as u32
                        {
                            info!(target: LOG_TARGET_APP_LOGIC, "Force-terminating group {} wrapper PID {}", group_idx, wrapper_pid);
                            TerminateProcess(handle, 1);
                        }

                        CloseHandle(handle);
                    } else {
                        warn!(target: LOG_TARGET_APP_LOGIC, "Failed to open handle for group {} wrapper PID {}: error={}", group_idx, wrapper_pid, std::io::Error::last_os_error());
                    }
                }
            }
        }

        #[cfg(not(windows))]
        {
            // On non-Windows, give processes a grace period to exit
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    pub fn is_running(&self) -> bool {
        !self.instances.is_empty() && !self.shutdown_signal.lock().unwrap().is_triggered()
    }

    /// Extract wrapper PIDs from all instances. Used by callers who need to terminate processes
    /// without holding a non-Send lock guard across await points.
    #[cfg(windows)]
    pub fn get_pids(&self) -> Vec<u32> {
        self.instances.iter().filter_map(|i| i.wrapper_pid).collect()
    }

    /// Extract (http_port, http_token) pairs from all instances for status aggregation
    /// without needing &mut access to the manager.
    pub fn get_ports(&self) -> Vec<(u16, String)> {
        self.instances.iter().map(|i| (i.http_port, i.http_token.clone())).collect()
    }
}
