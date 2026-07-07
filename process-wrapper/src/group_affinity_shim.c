// Copyright 2026. The Tari Project
// SPDX-License-Identifier: ECAPL-1.0

//! Minimal C shim for SetProcessGroupAffinity, which may not be exported by the
//! linker import library on all Windows SDK configurations. Load it dynamically
//! via GetProcAddress at runtime to avoid LNK2019.

#include <windows.h>

int shim_set_process_group_affinity(HANDLE hProcess, USHORT groupCount, GROUP_AFFINITY* affinity) {
    typedef BOOL (WINAPI *PFN_SetProcessGroupAffinity)(HANDLE, USHORT, PGROUP_AFFINITY);
    static PFN_SetProcessGroupAffinity fn = NULL;
    if (!fn) {
        fn = (PFN_SetProcessGroupAffinity)GetProcAddress(
            GetModuleHandleA("kernel32.dll"), "SetProcessGroupAffinity");
    }
    if (!fn) return 0;
    return fn(hProcess, groupCount, affinity);
}
