// Copyright 2026. The Tari Project
// SPDX-License-Identifier: ECAPL-1.0

//! Build script — compiles group_affinity_shim.c so we can call
//! SetProcessGroupAffinity even though windows-sys v0.52 doesn't expose it.

fn main() {
    #[cfg(windows)]
    {
        cc::Build::new()
            .file("src/group_affinity_shim.c")
            .compile("group_affinity_shim");

        println!("cargo::rustc-link-lib=kernel32");
    }
}
  
