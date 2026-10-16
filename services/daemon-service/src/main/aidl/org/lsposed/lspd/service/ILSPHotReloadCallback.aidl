package org.lsposed.lspd.service;

import org.lsposed.lspd.models.Module;

/**
 * Implemented by an injected process. The daemon calls {@link #hotReload} to replace the module
 * generation running in that process.
 */
interface ILSPHotReloadCallback {
    // Hot reload status codes, mirroring io.github.libxposed.service.IXposedService.HOT_RELOAD_*.
    const int HOT_RELOAD_SUCCEEDED = 0;
    const int HOT_RELOAD_FAILED = 1;
    const int HOT_RELOAD_UNSUPPORTED = 2;
    const int HOT_RELOAD_IN_PROGRESS = 3;
    const int HOT_RELOAD_PROCESS_DIED = 4;

    /**
     * Triggers a hot reload of the module in the calling process.
     *
     * @return a Bundle carrying {@code "status"} (one of the {@code HOT_RELOAD_*} codes above) and
     * an optional {@code "message"} diagnostic string.
     */
    Bundle hotReload(in Module module, in Bundle extras);
}
