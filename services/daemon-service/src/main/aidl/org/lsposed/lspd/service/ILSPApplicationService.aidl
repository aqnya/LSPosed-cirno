package org.lsposed.lspd.service;

import org.lsposed.lspd.models.Module;
import org.lsposed.lspd.service.ILSPHotReloadCallback;

interface ILSPApplicationService {
    List<Module> getLegacyModulesList();

    List<Module> getModulesList();

    String getPrefsPath(String packageName);

    ParcelFileDescriptor requestInjectedManagerBinder(out List<IBinder> binder);

    void registerHotReloadCallback(ILSPHotReloadCallback callback);
}
