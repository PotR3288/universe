// Copyright 2024. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the
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

//! Cross-platform available thread count with correct handling of systems that have
//! more than 64 logical processors (multiple Windows processor groups / NUMA nodes).
//!
//! On Windows, `std::thread::available_parallelism()` delegates to
//! `GetActiveProcessorCount(ALL_PROCESSOR_GROUPS)`. While this API was designed to
//! return the total across all groups, it can return only the first group's count
//! (capped at 64) when process affinity is restricted or on certain OS configurations.
//! Each Windows processor group caps at 64 CPUs, so systems with >64 threads span
//! multiple groups and need explicit iteration to get the true total.

/// Returns the number of available logical processors across **all** CPU groups / NUMA nodes.
///
/// On Windows this iterates every processor group via `GetActiveProcessorCount` and sums
/// them, avoiding the 64-thread cap that occurs when only the first group is reported.
pub fn available_parallelism() -> usize {
    #[cfg(windows)]
    {
        get_active_processor_count_all_groups()
    }

    #[cfg(not(windows))]
    {
        // On non-Windows platforms std's implementation is correct.
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or_else(|_| num_cpus_get())
    }
}

#[cfg(windows)]
fn get_active_processor_count_all_groups() -> usize {
    use windows_sys::Win32::System::Threading::{
        GetActiveProcessorCount, GetMaximumProcessorGroupCount,
    };

    unsafe {
        let max_groups = GetMaximumProcessorGroupCount();
        let mut total: u32 = 0;

        for group in 0..max_groups {
            let count = GetActiveProcessorCount(group);
            if count > 0 {
                total += count;
            }
        }

        // If the sum is zero something went very wrong — fall back to std.
        if total == 0 {
            return std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or_else(|_| num_cpus_get());
        }

        total as usize
    }
}

/// Minimal inline implementation of `num_cpus::get()` for the fallback path.
/// Avoids adding a crate dependency just for an error-case fallback on non-Windows.
fn num_cpus_get() -> usize {
    // Use sysinfo as a fallback — it's already a project dependency.
    sysinfo::System::new_all().cpus().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_parallelism_returns_positive() {
        let count = available_parallelism();
        assert!(
            count > 0,
            "available_parallelism should return a positive number"
        );
    }
}
