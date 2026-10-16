package org.lsposed.lspd.impl;

import android.annotation.SuppressLint;
import android.app.ActivityThread;
import android.content.SharedPreferences;
import android.content.pm.ApplicationInfo;
import android.os.Build;
import android.os.Bundle;
import android.os.DeadSystemException;
import android.os.ParcelFileDescriptor;
import android.os.Process;
import android.os.RemoteException;
import android.util.Log;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;

import org.lsposed.lspd.core.BuildConfig;
import org.lsposed.lspd.models.Module;
import org.lsposed.lspd.nativebridge.NativeAPI;
import org.lsposed.lspd.service.ILSPHotReloadCallback;
import org.lsposed.lspd.service.ILSPInjectedModuleService;
import org.lsposed.lspd.util.LspModuleClassLoader;

import java.io.File;
import java.io.FileNotFoundException;
import java.lang.reflect.Constructor;
import java.lang.reflect.Executable;
import java.lang.reflect.Method;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.net.UnknownHostException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import io.github.libxposed.api.XposedInterface;
import io.github.libxposed.api.XposedModule;
import io.github.libxposed.api.XposedModuleInterface;
import io.github.libxposed.api.error.XposedFrameworkError;


@SuppressLint("NewApi")
public class LSPosedContext implements XposedInterface {

    private static final String TAG = "LSPosedContext";

    public static boolean isSystemServer;
    public static String appDir;
    public static String processName;

    static final Set<XposedModule> modules = ConcurrentHashMap.newKeySet();

    // The framework interface (generation) each loaded entry belongs to.
    private static final Map<XposedModule, LSPosedContext> sEntryContexts = new ConcurrentHashMap<>();
    // Module packages currently being hot-reloaded, to serialize reloads per package.
    private static final Set<String> sReloading = ConcurrentHashMap.newKeySet();

    private final String mPackageName;
    private final ApplicationInfo mApplicationInfo;
    private final ILSPInjectedModuleService service;
    private final ExceptionMode mDefaultExceptionMode;
    private final Map<String, SharedPreferences> mRemotePrefs = new ConcurrentHashMap<>();
    private ClassLoader mClassLoader;
    // Set while the module is being hot-reloaded: further hook registrations from this generation fail.
    volatile boolean frozen = false;

    LSPosedContext(String packageName, ApplicationInfo applicationInfo, ILSPInjectedModuleService service,
                   ExceptionMode defaultExceptionMode) {
        this.mPackageName = packageName;
        this.mApplicationInfo = applicationInfo;
        this.service = service;
        this.mDefaultExceptionMode = defaultExceptionMode;
    }

    boolean isFrozen() {
        return frozen;
    }

    public static void callOnPackageLoaded(XposedModuleInterface.PackageLoadedParam param) {
        for (XposedModule module : modules) {
            try {
                module.onPackageLoaded(param);
            } catch (Throwable t) {
                Log.e(TAG, "Error when calling onPackageLoaded of " + module.getModuleApplicationInfo().packageName, t);
            }
        }
    }

    public static void callOnPackageReady(XposedModuleInterface.PackageReadyParam param) {
        for (XposedModule module : modules) {
            try {
                module.onPackageReady(param);
            } catch (Throwable t) {
                Log.e(TAG, "Error when calling onPackageReady of " + module.getModuleApplicationInfo().packageName, t);
            }
        }
    }

    /**
     * Stops the framework from delivering further lifecycle callbacks to the given module entry.
     * Invoked by {@link XposedModule#detach()}.
     */
    public static void detachModule(XposedModule module) {
        modules.remove(module);
        sEntryContexts.remove(module);
    }

    public static void callOnSystemServerStarting(XposedModuleInterface.SystemServerStartingParam param) {
        for (XposedModule module : modules) {
            try {
                module.onSystemServerStarting(param);
            } catch (Throwable t) {
                Log.e(TAG, "Error when calling onSystemServerStarting of " + module.getModuleApplicationInfo().packageName, t);
            }
        }
    }

    private static final class Generation {
        final LSPosedContext context;
        final List<XposedModule> entries = new ArrayList<>();

        Generation(LSPosedContext context) {
            this.context = context;
        }
    }

    private static XposedModuleInterface.ModuleLoadedParam moduleLoadedParam() {
        return new XposedModuleInterface.ModuleLoadedParam() {
            @Override
            public boolean isSystemServer() {
                return isSystemServer;
            }

            @NonNull
            @Override
            public String getProcessName() {
                return processName;
            }
        };
    }

    /**
     * Builds a new module generation: a fresh classloader and framework context, with all entry
     * classes instantiated and attached, but without dispatching any lifecycle callback.
     */
    private static Generation loadGeneration(Module module) throws Throwable {
        var sb = new StringBuilder();
        var abis = Process.is64Bit() ? Build.SUPPORTED_64_BIT_ABIS : Build.SUPPORTED_32_BIT_ABIS;
        for (String abi : abis) {
            sb.append(module.apkPath).append("!/lib/").append(abi).append(File.pathSeparator);
        }
        var librarySearchPath = sb.toString();
        var initLoader = XposedModule.class.getClassLoader();
        var mcl = LspModuleClassLoader.loadApk(module.apkPath, module.file.preLoadedDexes, librarySearchPath, initLoader);
        if (mcl.loadClass(XposedModule.class.getName()).getClassLoader() != initLoader) {
            Log.e(TAG, "  Cannot load module: " + module.packageName);
            Log.e(TAG, "  The Xposed API classes are compiled into the module's APK.");
            Log.e(TAG, "  This may cause strange issues and must be fixed by the module developer.");
            throw new IllegalStateException("The Xposed API classes are compiled into the module's APK");
        }
        module.file.moduleLibraryNames.forEach(NativeAPI::recordNativeEntrypoint);
        var defaultExceptionMode = module.file.exceptionPassthrough ? ExceptionMode.PASSTHROUGH : ExceptionMode.PROTECTIVE;
        var ctx = new LSPosedContext(module.packageName, module.applicationInfo, module.service, defaultExceptionMode);
        ctx.mClassLoader = mcl;
        var generation = new Generation(ctx);
        for (var entry : module.file.moduleClassNames) {
            var moduleClass = mcl.loadClass(entry);
            Log.d(TAG, "  Loading class " + moduleClass);
            if (!XposedModule.class.isAssignableFrom(moduleClass)) {
                Log.e(TAG, "    This class doesn't implement any sub-interface of XposedModule, skipping it");
                continue;
            }
            try {
                var moduleContext = (XposedModule) moduleClass.getConstructor().newInstance();
                moduleContext.attachFramework(ctx, () -> detachModule(moduleContext));
                generation.entries.add(moduleContext);
            } catch (Throwable e) {
                Log.e(TAG, "    Failed to load class " + moduleClass, e);
            }
        }
        return generation;
    }

    @SuppressLint("DiscouragedPrivateApi")
    public static boolean loadModule(ActivityThread at, Module module) {
        try {
            Log.d(TAG, "Loading module " + module.packageName);
            var generation = loadGeneration(module);
            var param = moduleLoadedParam();
            for (var entry : generation.entries) {
                entry.onModuleLoaded(param);
                modules.add(entry);
                sEntryContexts.put(entry, generation.context);
            }
            Log.d(TAG, "Loaded module " + module.packageName + ": " + generation.context);
        } catch (Throwable e) {
            Log.d(TAG, "Loading module " + module.packageName, e);
            return false;
        }
        return true;
    }

    /**
     * Replaces the running module generation with the one described by {@code newModule}. Invoked
     * through {@link LSPosedHotReloadCallback} in the target process.
     *
     * @return a Bundle with {@code "status"} (one of the {@code ILSPHotReloadCallback.HOT_RELOAD_*} codes)
     * and an optional {@code "message"}.
     */
    public static Bundle handleHotReload(Module newModule, Bundle extras) {
        var packageName = newModule.packageName;
        if (!sReloading.add(packageName)) {
            return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_IN_PROGRESS, null);
        }
        try {
            var reloadable = newModule.file != null && !newModule.file.legacy
                    && newModule.file.moduleClassNames.size() == 1;
            var oldEntry = findEntry(packageName);
            var oldCtx = oldEntry == null ? null : sEntryContexts.get(oldEntry);
            if (!reloadable || oldEntry == null || oldCtx == null) {
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_UNSUPPORTED,
                        "The module is not hot-reloadable in this process");
            }
            if (countEntries(packageName) != 1) {
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_UNSUPPORTED,
                        "Only modules with a single entry class can be hot-reloaded");
            }

            oldCtx.frozen = true;
            var oldHandles = LSPosedBridge.captureHandles(oldCtx);
            var savedStateHolder = new Object[1];
            boolean proceed;
            try {
                proceed = oldEntry.onHotReloading(
                        new HotReloadingParamImpl(extras, savedStateHolder, oldCtx.mClassLoader));
            } catch (Throwable t) {
                oldCtx.frozen = false;
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_FAILED, Log.getStackTraceString(t));
            }
            if (!proceed) {
                oldCtx.frozen = false;
                // A null message signals that the old module refused the reload.
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_FAILED, null);
            }

            Generation generation;
            try {
                generation = loadGeneration(newModule);
            } catch (Throwable t) {
                oldCtx.frozen = false;
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_FAILED, Log.getStackTraceString(t));
            }
            if (generation.entries.size() != 1) {
                oldCtx.frozen = false;
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_UNSUPPORTED,
                        "Only modules with a single entry class can be hot-reloaded");
            }

            var newEntry = generation.entries.get(0);
            try {
                newEntry.onHotReloaded(new HotReloadedParamImpl(
                        extras, savedStateHolder[0], oldHandles));
            } catch (Throwable t) {
                oldCtx.frozen = false;
                return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_FAILED, Log.getStackTraceString(t));
            }

            // The new generation took over; drop every framework reference to the old one.
            modules.remove(oldEntry);
            sEntryContexts.remove(oldEntry);
            modules.add(newEntry);
            sEntryContexts.put(newEntry, generation.context);
            Log.d(TAG, "Hot reloaded module " + packageName);
            return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_SUCCEEDED, null);
        } catch (Throwable t) {
            Log.e(TAG, "Hot reload failed for " + packageName, t);
            return hotReloadResult(ILSPHotReloadCallback.HOT_RELOAD_FAILED, Log.getStackTraceString(t));
        } finally {
            sReloading.remove(packageName);
        }
    }

    private static Bundle hotReloadResult(int status, @Nullable String message) {
        var bundle = new Bundle();
        bundle.putInt("status", status);
        if (message != null) bundle.putString("message", message);
        return bundle;
    }

    private static XposedModule findEntry(String packageName) {
        for (var entry : modules) {
            if (packageName.equals(packageNameOf(entry))) return entry;
        }
        return null;
    }

    private static long countEntries(String packageName) {
        return modules.stream().filter(entry -> packageName.equals(packageNameOf(entry))).count();
    }

    private static String packageNameOf(XposedModule entry) {
        try {
            return entry.getModuleApplicationInfo().packageName;
        } catch (Throwable e) {
            return null;
        }
    }

    private static final class HotReloadingParamImpl implements XposedModuleInterface.HotReloadingParam {
        private final Bundle extras;
        private final Object[] savedState;
        private final ClassLoader oldClassLoader;

        HotReloadingParamImpl(Bundle extras, Object[] savedState, ClassLoader oldClassLoader) {
            this.extras = extras;
            this.savedState = savedState;
            this.oldClassLoader = oldClassLoader;
        }

        @Nullable
        @Override
        public Bundle getExtras() {
            return extras;
        }

        @Override
        public void setSavedInstanceState(@Nullable Object outState) {
            if (outState != null && oldClassLoader != null) {
                for (var cl = outState.getClass().getClassLoader(); cl != null; cl = cl.getParent()) {
                    if (cl == oldClassLoader) {
                        throw new IllegalArgumentException(
                                "Saved state must not reference the old module classloader");
                    }
                }
            }
            savedState[0] = outState;
        }
    }

    private static final class HotReloadedParamImpl implements XposedModuleInterface.HotReloadedParam {
        private final Bundle extras;
        private final Object savedState;
        private final List<XposedInterface.HookHandle> oldHandles;

        HotReloadedParamImpl(Bundle extras, Object savedState,
                             List<XposedInterface.HookHandle> oldHandles) {
            this.extras = extras;
            this.savedState = savedState;
            this.oldHandles = oldHandles;
        }

        @Nullable
        @Override
        public Bundle getExtras() {
            return extras;
        }

        @Nullable
        @Override
        public Object getSavedInstanceState() {
            return savedState;
        }

        @NonNull
        @Override
        public List<XposedInterface.HookHandle> getOldHookHandles() {
            return oldHandles;
        }

        @Override
        public boolean isSystemServer() {
            return isSystemServer;
        }

        @NonNull
        @Override
        public String getProcessName() {
            return processName;
        }
    }

    @NonNull
    @Override
    public String getFrameworkName() {
        return BuildConfig.FRAMEWORK_NAME;
    }

    @NonNull
    @Override
    public String getFrameworkVersion() {
        return BuildConfig.VERSION_NAME;
    }

    @Override
    public long getFrameworkVersionCode() {
        return BuildConfig.VERSION_CODE;
    }

    @Override
    public long getFrameworkProperties() {
        try {
            return service.getFrameworkProperties();
        } catch (RemoteException e) {
            throw new XposedFrameworkError(e);
        }
    }

    @Override
    @NonNull
    public HookBuilder hook(@NonNull Executable origin) {
        return LSPosedBridge.newHookBuilder(this, origin, mDefaultExceptionMode);
    }

    @Override
    @NonNull
    public HookBuilder hookClassInitializer(@NonNull Class<?> origin) {
        return LSPosedBridge.newClassInitializerHookBuilder(this, origin, mDefaultExceptionMode);
    }

    @Override
    public boolean deoptimize(@NonNull Executable executable) {
        return LSPosedBridge.doDeoptimize(executable);
    }

    @NonNull
    @Override
    public Invoker<?, Method> getInvoker(@NonNull Method method) {
        return LSPosedBridge.newInvoker(method);
    }

    @NonNull
    @Override
    public <T> CtorInvoker<T> getInvoker(@NonNull Constructor<T> constructor) {
        return LSPosedBridge.newInvoker(constructor);
    }

    @Override
    public void log(int priority, @Nullable String tag, @NonNull String msg) {
        log(priority, tag, msg, null);
    }

    @Override
    public void log(int priority, @Nullable String tag, @NonNull String message, @Nullable Throwable throwable) {
        if (message.isEmpty() && throwable == null) {
            return;
        }

        var estimatedLength = Math.max(0xFC2 - (tag == null ? 0 : tag.length()), 100);
        var output = new StringWriter(estimatedLength);
        var writer = new PrintWriter(output);

        var moduleTag = String.valueOf(tag);
        writer.println(String.format("[%s,%s] %s", mPackageName, moduleTag, message));

        if (throwable != null) {
            Throwable candidate;
            for (candidate = throwable; candidate != null && !(candidate instanceof UnknownHostException); candidate = candidate.getCause()) {
                if (candidate instanceof DeadSystemException) {
                    writer.println("DeadSystemException: The system died; earlier logs will point to the root cause");
                    break;
                }
            }
            if (candidate == null) {
                throwable.printStackTrace(writer);
            }
        }

        writer.flush();
        Log.println(priority, TAG, output.toString());
    }

    @NonNull
    @Override
    public ApplicationInfo getModuleApplicationInfo() {
        return mApplicationInfo;
    }

    @NonNull
    @Override
    public SharedPreferences getRemotePreferences(String name) {
        if (name == null) throw new IllegalArgumentException("name must not be null");
        return mRemotePrefs.computeIfAbsent(name, n -> {
            try {
                return new LSPosedRemotePreferences(service, n);
            } catch (RemoteException e) {
                log(Log.ERROR, "null", "Failed to get remote preferences", e);
                throw new XposedFrameworkError(e);
            }
        });
    }

    @NonNull
    @Override
    public String[] listRemoteFiles() {
        try {
            return service.getRemoteFileList();
        } catch (RemoteException e) {
            log(Log.ERROR, "null", "Failed to list remote files", e);
            throw new XposedFrameworkError(e);
        }
    }

    @NonNull
    @Override
    public ParcelFileDescriptor openRemoteFile(String name) throws FileNotFoundException {
        if (name == null) throw new IllegalArgumentException("name must not be null");
        try {
            return service.openRemoteFile(name);
        } catch (RemoteException e) {
            throw new FileNotFoundException(e.getMessage());
        }
    }
}
