/*
 * This file is part of LSPosed.
 *
 * LSPosed is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * LSPosed is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with LSPosed.  If not, see <https://www.gnu.org/licenses/>.
 *
 * Copyright (C) 2021 LSPosed Contributors
 */

package org.lsposed.lspd.service;

import static org.lsposed.lspd.service.PackageService.PER_USER_RANGE;

import android.content.AttributionSource;
import android.os.Binder;
import android.os.Build;
import android.os.Bundle;
import android.os.DeadObjectException;
import android.os.ParcelFileDescriptor;
import android.os.Process;
import android.os.RemoteException;
import android.util.ArrayMap;
import android.util.Log;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;

import org.lsposed.daemon.BuildConfig;
import org.lsposed.lspd.models.Module;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.WeakHashMap;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

import io.github.libxposed.api.XposedInterface;
import io.github.libxposed.service.HookedProcess;
import io.github.libxposed.service.IHotReloadCallback;
import io.github.libxposed.service.IXposedScopeCallback;
import io.github.libxposed.service.IXposedService;

public class LSPModuleService extends IXposedService.Stub {

    private final static String TAG = "LSPosedModuleService";

    private final static Set<Integer> uidSet = ConcurrentHashMap.newKeySet();
    private final static Set<ModuleBinderKey> sentBinderSet = ConcurrentHashMap.newKeySet();
    private final static Set<ModuleBinderKey> sendingBinderSet = ConcurrentHashMap.newKeySet();
    private final static Map<Module, LSPModuleService> serviceMap = Collections.synchronizedMap(new WeakHashMap<>());
    private final static ExecutorService binderExecutor = Executors.newSingleThreadExecutor(r -> new Thread(r, "module-binder-delivery"));

    // API 102: opaque target ids handed out by getRunningTargets() and expected back by hotReloadModule().
    private final static AtomicLong sNextTargetId = new AtomicLong(1);
    private final static Map<Long, ProcessRef> sTargets = new ConcurrentHashMap<>();
    // Targets that currently have a hot reload in flight.
    private final static Set<ProcessRef> sReloadingTargets = ConcurrentHashMap.newKeySet();
    private final static ExecutorService hotReloadExecutor =
            Executors.newSingleThreadExecutor(r -> new Thread(r, "module-hot-reload"));

    static final int XPOSED_API_VERSION = XposedInterface.LIB_API;

    public final static String FILES_DIR = "files";

    private record ProcessRef(int uid, int pid, String processName) {
    }

    private final @NonNull
    Module loadedModule;

    static void uidClear() {
        uidSet.clear();
        sentBinderSet.clear();
        sendingBinderSet.clear();
        sTargets.clear();
        sReloadingTargets.clear();
    }

    static void uidStarts(int uid) {
        if (uidSet.add(uid)) {
            sendBinderForUid(uid);
        }
    }

    static void uidGone(int uid) {
        uidSet.remove(uid);
        sentBinderSet.removeIf(k -> k.uid == uid);
        sendingBinderSet.removeIf(k -> k.uid == uid);
        sTargets.entrySet().removeIf(e -> e.getValue().uid() == uid);
        sReloadingTargets.removeIf(ref -> ref.uid() == uid);
    }

    static void sendBindersForRunningModules() {
        for (int uid : uidSet) {
            sendBinderForUid(uid);
        }
    }

    static void sendBinderForRunningModule(String packageName) {
        for (int uid : uidSet) {
            var module = ConfigManager.getInstance().getModule(uid);
            if (module != null && Objects.equals(module.packageName, packageName)) {
                sendBinderForModule(module, uid);
            }
        }
    }

    private static void sendBinderForUid(int uid) {
        var module = ConfigManager.getInstance().getModule(uid);
        if (module != null) {
            sendBinderForModule(module, uid);
        }
    }

    private static void sendBinderForModule(Module module, int uid) {
        if (module.file == null || module.file.legacy) {
            return;
        }
        var key = new ModuleBinderKey(module.packageName, uid);
        if (sentBinderSet.contains(key) || !sendingBinderSet.add(key)) {
            return;
        }
        try {
            LSPModuleService service;
            synchronized (serviceMap) {
                service = serviceMap.computeIfAbsent(module, LSPModuleService::new);
            }
            binderExecutor.execute(() -> service.sendBinder(uid, key));
        } catch (Throwable e) {
            sendingBinderSet.remove(key);
            Log.w(TAG, "failed to schedule module binder for uid " + uid, e);
        }
    }

    private void sendBinder(int uid, ModuleBinderKey key) {
        var name = loadedModule.packageName;
        try {
            int userId = uid / PackageService.PER_USER_RANGE;
            if (!ConfigManager.getInstance().isModuleEnabledForUser(name, userId)) {
                return;
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                try {
                    DeviceIdleService.addPowerSaveTempWhitelistApp(name, userId, "shell");
                    Log.d(TAG, "add " + userId + ":" + name + " to power save temp whitelist for 30s");
                    try {
                        Thread.sleep(400L);
                    } catch (InterruptedException e) {
                        Log.d(TAG, "sendBinder interrupted while waiting for whitelist, continuing for " + name, e);
                    }
                } catch (Throwable e) {
                    Log.e(TAG, "failed to add " + userId + ":" + name + " to power save temp whitelist", e);
                }
            }
            var authority = name + AUTHORITY_SUFFIX;
            var provider = ActivityManagerService.getContentProvider(authority, userId);
            for (int attempt = 1; provider == null && attempt < 3; attempt++) {
                Log.d(TAG, "no service provider for " + name + ", attempt " + attempt);
                try {
                    Thread.sleep(1000L);
                } catch (InterruptedException e) {
                    Log.d(TAG, "sendBinder interrupted during retry sleep for " + name + ", continuing", e);
                }
                provider = ActivityManagerService.getContentProvider(authority, userId);
            }
            if (provider == null) {
                Log.d(TAG, "no service provider for " + name + " after 3 attempts");
                return;
            }
            var extra = new Bundle();
            extra.putBinder("binder", asBinder());
            Bundle reply = null;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                reply = provider.call(new AttributionSource.Builder(1000).setPackageName("android").build(), authority, SEND_BINDER, null, extra);
            } else if (Build.VERSION.SDK_INT == Build.VERSION_CODES.R) {
                reply = provider.call("android", null, authority, SEND_BINDER, null, extra);
            } else if (Build.VERSION.SDK_INT == Build.VERSION_CODES.Q) {
                reply = provider.call("android", authority, SEND_BINDER, null, extra);
            } else {
                reply = provider.call("android", SEND_BINDER, null, extra);
            }
            if (reply != null) {
                Log.d(TAG, "sent module binder to " + name);
                sentBinderSet.add(key);
            } else {
                Log.w(TAG, "failed to send module binder to " + name);
            }
        } catch (Throwable e) {
            Log.w(TAG, "failed to send module binder for uid " + uid, e);
        } finally {
            sendingBinderSet.remove(key);
        }
    }

    private static final class ModuleBinderKey {
        private final String packageName;
        private final int uid;

        private ModuleBinderKey(String packageName, int uid) {
            this.packageName = packageName;
            this.uid = uid;
        }

        @Override
        public boolean equals(Object o) {
            if (!(o instanceof ModuleBinderKey)) return false;
            var key = (ModuleBinderKey) o;
            return uid == key.uid && Objects.equals(packageName, key.packageName);
        }

        @Override
        public int hashCode() {
            return Objects.hash(packageName, uid);
        }
    }

    LSPModuleService(@NonNull Module module) {
        loadedModule = module;
    }

    private int ensureModule() throws RemoteException {
        var appId = Binder.getCallingUid() % PER_USER_RANGE;
        if (loadedModule.appId != appId) {
            throw new RemoteException("Module " + loadedModule.packageName + " is not for uid " + Binder.getCallingUid());
        }
        return Binder.getCallingUid() / PER_USER_RANGE;
    }

    @Override
    public int getApiVersion() throws RemoteException {
        ensureModule();
        return XPOSED_API_VERSION;
    }

    @Override
    public String getFrameworkName() throws RemoteException {
        ensureModule();
        return "LSPosed";
    }

    @Override
    public String getFrameworkVersion() throws RemoteException {
        ensureModule();
        return BuildConfig.VERSION_NAME;
    }

    @Override
    public long getFrameworkVersionCode() throws RemoteException {
        ensureModule();
        return BuildConfig.VERSION_CODE;
    }

    @Override
    public long getFrameworkProperties() throws RemoteException {
        ensureModule();
        var properties = IXposedService.PROP_CAP_SYSTEM | IXposedService.PROP_CAP_REMOTE;
        if (ConfigManager.getInstance().dexObfuscate()) {
            properties |= IXposedService.PROP_RT_API_PROTECTION;
        }
        return properties;
    }

    @Override
    public List<String> getScope() throws RemoteException {
        ensureModule();
        ArrayList<String> res = new ArrayList<>();
        var scope = ConfigManager.getInstance().getModuleScope(loadedModule.packageName);
        if (scope == null) return res;
        for (var s : scope) {
            res.add(s.packageName);
        }
        return res;
    }

    @Override
    public void requestScope(List<String> packages, IXposedScopeCallback callback) throws RemoteException {
        Objects.requireNonNull(packages, "packages cannot be null");
        Objects.requireNonNull(callback, "callback cannot be null");
        var userId = ensureModule();
        var scopePackages = new ArrayList<>(packages);
        if (scopePackages.isEmpty()) {
            callback.onScopeRequestFailed("Invalid request");
            return;
        }
        if (ConfigManager.getInstance().scopeRequestBlocked(loadedModule.packageName, userId)) {
            callback.onScopeRequestFailed("Blocked by user");
        } else {
            LSPNotificationManager.requestModuleScope(loadedModule.packageName, userId, scopePackages, callback);
        }
    }

    @Override
    public void removeScope(List<String> packages) throws RemoteException {
        Objects.requireNonNull(packages, "packages cannot be null");
        var userId = ensureModule();
        try {
            if (!ConfigManager.getInstance().removeModuleScope(loadedModule.packageName, packages, userId)) {
                throw new RemoteException("Invalid request");
            }
        } catch (Throwable e) {
            throw new RemoteException(e.getMessage());
        }
    }

    @Override
    public List<HookedProcess> getRunningTargets() throws RemoteException {
        var userId = ensureModule();
        var config = ConfigManager.getInstance();
        var result = new ArrayList<HookedProcess>();
        for (var process : LSPApplicationService.getRunningProcesses()) {
            if (process.uid / PER_USER_RANGE != userId) continue;
            if (!isModuleInjected(config, process.uid, process.processName)) continue;
            var targetId = sNextTargetId.getAndIncrement();
            sTargets.put(targetId, new ProcessRef(process.uid, process.pid, process.processName));
            var hooked = new HookedProcess();
            hooked.targetId = targetId;
            hooked.uid = process.uid;
            hooked.pid = process.pid;
            hooked.processName = process.processName;
            hooked.state = HookedProcess.TARGET_STATE_UP_TO_DATE;
            hooked.loadedVersionCode = loadedModuleVersionCode();
            result.add(hooked);
        }
        return result;
    }

    @Override
    public void hotReloadModule(long targetId, @Nullable Bundle data, IHotReloadCallback callback) throws RemoteException {
        ensureModule();
        Objects.requireNonNull(callback, "callback cannot be null");
        var ref = sTargets.get(targetId);
        if (ref == null) {
            throw new SecurityException("Invalid or expired target");
        }
        var config = ConfigManager.getInstance();
        // The target must still be alive, still injected, and still belong to this module.
        var processInfo = LSPApplicationService.getProcess(ref.uid(), ref.pid());
        if (processInfo == null || !isModuleInjected(config, ref.uid(), ref.processName())) {
            sTargets.remove(targetId);
            throw new SecurityException("Invalid or expired target");
        }
        var hotReloadCallback = processInfo.hotReloadCallback;
        if (hotReloadCallback == null) {
            callback.onHotReloadResult(IXposedService.HOT_RELOAD_UNSUPPORTED,
                    "The target process does not support hot reload");
            return;
        }
        if (!sReloadingTargets.add(ref)) {
            callback.onHotReloadResult(IXposedService.HOT_RELOAD_IN_PROGRESS, null);
            return;
        }
        var module = moduleSnapshotForReload();
        if (module == null) {
            sReloadingTargets.remove(ref);
            callback.onHotReloadResult(IXposedService.HOT_RELOAD_UNSUPPORTED,
                    "The module is not hot-reloadable");
            return;
        }
        // Reloading blocks on module code, so run it off the binder thread. The result is reported
        // asynchronously through the client callback.
        hotReloadExecutor.execute(() -> {
            var status = IXposedService.HOT_RELOAD_FAILED;
            String message = null;
            try {
                var result = hotReloadCallback.hotReload(module, data);
                if (result != null) {
                    status = result.getInt("status", IXposedService.HOT_RELOAD_FAILED);
                    message = result.getString("message");
                } else {
                    status = IXposedService.HOT_RELOAD_UNSUPPORTED;
                }
            } catch (DeadObjectException e) {
                status = IXposedService.HOT_RELOAD_PROCESS_DIED;
            } catch (RemoteException e) {
                status = IXposedService.HOT_RELOAD_PROCESS_DIED;
            } catch (Throwable t) {
                status = IXposedService.HOT_RELOAD_FAILED;
                message = t.toString();
            } finally {
                sReloadingTargets.remove(ref);
            }
            try {
                callback.onHotReloadResult(status, message);
            } catch (RemoteException ignored) {
            }
        });
    }

    /**
     * Builds a fresh snapshot of the module as currently installed on disk, or {@code null} when
     * the module cannot be hot-reloaded.
     */
    @Nullable
    private Module moduleSnapshotForReload() {
        try {
            var file = ConfigFileManager.loadModule(loadedModule.apkPath, ConfigManager.getInstance().dexObfuscate());
            if (file == null || file.legacy || file.moduleClassNames.size() != 1) return null;
            var module = new Module();
            module.packageName = loadedModule.packageName;
            module.appId = loadedModule.appId;
            module.apkPath = loadedModule.apkPath;
            module.applicationInfo = loadedModule.applicationInfo;
            module.service = loadedModule.service;
            module.file = file;
            return module;
        } catch (Throwable t) {
            Log.w(TAG, "failed to snapshot module for hot reload", t);
            return null;
        }
    }

    private boolean isModuleInjected(ConfigManager config, int uid, String processName) {
        List<Module> modules;
        if (uid == Process.SYSTEM_UID && "system".equals(processName)) {
            modules = config.getModulesForSystemServer();
        } else {
            modules = config.getModulesForProcess(processName, uid);
        }
        if (modules == null) return false;
        for (var module : modules) {
            if (Objects.equals(module.packageName, loadedModule.packageName)) return true;
        }
        return false;
    }

    private long loadedModuleVersionCode() {
        var info = loadedModule.applicationInfo;
        if (info == null) return 0L;
        // versionCode / longVersionCode are absent from the hidden-API compile stubs, so read them
        // reflectively. The runtime ApplicationInfo always exposes them.
        try {
            return info.getClass().getField("longVersionCode").getLong(info);
        } catch (Throwable ignored) {
        }
        try {
            return info.getClass().getField("versionCode").getInt(info);
        } catch (Throwable ignored) {
        }
        return 0L;
    }

    @Override
    public Bundle requestRemotePreferences(String group) throws RemoteException {
        var userId = ensureModule();
        var bundle = new Bundle();
        bundle.putSerializable("map", ConfigManager.getInstance().getModulePrefs(loadedModule.packageName, userId, group));
        return bundle;
    }

    @Override
    public void updateRemotePreferences(String group, Bundle diff) throws RemoteException {
        var userId = ensureModule();
        Map<String, Object> values = new ArrayMap<>();
        if (diff.containsKey("delete")) {
            var deletes = (Set<?>) diff.getSerializable("delete");
            for (var key : deletes) {
                values.put((String) key, null);
            }
        }
        if (diff.containsKey("put")) {
            try {
                var puts = (Map<?, ?>) diff.getSerializable("put");
                for (var entry : puts.entrySet()) {
                    values.put((String) entry.getKey(), entry.getValue());
                }
            } catch (Throwable e) {
                Log.e(TAG, "updateRemotePreferences: ", e);
            }
        }
        try {
            ConfigManager.getInstance().updateModulePrefs(loadedModule.packageName, userId, group, values);
            ((LSPInjectedModuleService) loadedModule.service).onUpdateRemotePreferences(userId, group, diff);
        } catch (Throwable e) {
            throw new RemoteException(e.getMessage());
        }
    }

    @Override
    public void deleteRemotePreferences(String group) throws RemoteException {
        var userId = ensureModule();
        ConfigManager.getInstance().deleteModulePrefs(loadedModule.packageName, userId, group);
    }

    @Override
    public String[] listRemoteFiles() throws RemoteException {
        var userId = ensureModule();
        try {
            var dir = ConfigFileManager.resolveModuleDir(loadedModule.packageName, FILES_DIR, userId, Binder.getCallingUid());
            var files = dir.toFile().list();
            return files == null ? new String[0] : files;
        } catch (IOException e) {
            throw new RemoteException(e.getMessage());
        }
    }

    @Override
    public ParcelFileDescriptor openRemoteFile(String path) throws RemoteException {
        var userId = ensureModule();
        ConfigFileManager.ensureModuleFilePath(path);
        try {
            var dir = ConfigFileManager.resolveModuleDir(loadedModule.packageName, FILES_DIR, userId, Binder.getCallingUid());
            return ParcelFileDescriptor.open(dir.resolve(path).toFile(), ParcelFileDescriptor.MODE_CREATE | ParcelFileDescriptor.MODE_READ_WRITE);
        } catch (IOException e) {
            throw new RemoteException(e.getMessage());
        }
    }

    @Override
    public boolean deleteRemoteFile(String path) throws RemoteException {
        var userId = ensureModule();
        ConfigFileManager.ensureModuleFilePath(path);
        try {
            var dir = ConfigFileManager.resolveModuleDir(loadedModule.packageName, FILES_DIR, userId, Binder.getCallingUid());
            return dir.resolve(path).toFile().delete();
        } catch (IOException e) {
            throw new RemoteException(e.getMessage());
        }
    }
}
