package org.lsposed.lspd.impl;

import android.util.Log;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;

import org.lsposed.lspd.nativebridge.HookBridge;

import java.lang.reflect.Constructor;
import java.lang.reflect.Executable;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Objects;
import java.util.Set;
import java.util.WeakHashMap;
import java.util.concurrent.ConcurrentHashMap;

import io.github.libxposed.api.XposedInterface;
import io.github.libxposed.api.error.HookFailedError;

public class LSPosedBridge {

    private static final String TAG = "LSPosed-Bridge";
    private static final String CAST_ERROR = "Return value's type from hook callback does not match the hooked method";
    private static final Object[] EMPTY_ARRAY = new Object[0];

    /**
     * Registry of hooks with an explicit id, isolated per module ({@link XposedInterface} instance).
     * A new hook created with the same id on the same executable atomically replaces the old one.
     */
    private static final WeakHashMap<XposedInterface, ConcurrentHashMap<Executable, ConcurrentHashMap<String, HookHandleImpl>>> sHooksById =
            new WeakHashMap<>();

    /**
     * Live hook handles created through {@link XposedInterface.HookBuilder}, per module. Used to
     * hand the previous generation's handles to the new generation during hot reload.
     */
    private static final WeakHashMap<XposedInterface, Set<HookHandleImpl>> sLiveHandles = new WeakHashMap<>();

    private static final XposedInterface.Hooker[] EMPTY_HOOKERS = new XposedInterface.Hooker[0];

    /**
     * Java-side mirror of the native hook table. Keeping the ordered hooker array here avoids a
     * JNI call (and a fresh {@code Object[]}) on every hooked invocation.
     */
    private static final ConcurrentHashMap<Executable, HookList> sHookLists = new ConcurrentHashMap<>();

    private static final class HookList {
        /** Immutable ordered view, published atomically so readers never see mixed arrays. */
        private static final class Snapshot {
            static final Snapshot EMPTY = new Snapshot(new int[0], EMPTY_HOOKERS);

            final int[] priorities;
            final XposedInterface.Hooker[] hookers;

            Snapshot(int[] priorities, XposedInterface.Hooker[] hookers) {
                this.priorities = priorities;
                this.hookers = hookers;
            }
        }

        private volatile Snapshot current = Snapshot.EMPTY;

        synchronized void add(int priority, XposedInterface.Hooker hooker) {
            var snapshot = current;
            var oldPriorities = snapshot.priorities;
            var oldHookers = snapshot.hookers;
            var length = oldHookers.length;

            // Keep the list sorted by descending priority, stable for equal priorities.
            var index = length;
            for (var i = 0; i < length; i++) {
                if (oldPriorities[i] < priority) {
                    index = i;
                    break;
                }
            }

            var newPriorities = new int[length + 1];
            var newHookers = new XposedInterface.Hooker[length + 1];
            System.arraycopy(oldPriorities, 0, newPriorities, 0, index);
            System.arraycopy(oldHookers, 0, newHookers, 0, index);
            newPriorities[index] = priority;
            newHookers[index] = hooker;
            System.arraycopy(oldPriorities, index, newPriorities, index + 1, length - index);
            System.arraycopy(oldHookers, index, newHookers, index + 1, length - index);
            current = new Snapshot(newPriorities, newHookers);
        }

        synchronized void remove(XposedInterface.Hooker hooker) {
            var snapshot = current;
            var oldPriorities = snapshot.priorities;
            var oldHookers = snapshot.hookers;
            for (var i = 0; i < oldHookers.length; i++) {
                if (oldHookers[i] == hooker) {
                    var newPriorities = new int[oldPriorities.length - 1];
                    var newHookers = new XposedInterface.Hooker[oldHookers.length - 1];
                    System.arraycopy(oldPriorities, 0, newPriorities, 0, i);
                    System.arraycopy(oldHookers, 0, newHookers, 0, i);
                    System.arraycopy(oldPriorities, i + 1, newPriorities, i, newPriorities.length - i);
                    System.arraycopy(oldHookers, i + 1, newHookers, i, newHookers.length - i);
                    current = new Snapshot(newPriorities, newHookers);
                    return;
                }
            }
        }

        XposedInterface.Hooker[] hookers(int maxPriority) {
            var snapshot = current;
            var all = snapshot.hookers;
            // Hot path: every interception asks for all hookers, so skip the priority check.
            if (maxPriority >= XposedInterface.PRIORITY_HIGHEST) return all;
            if (all.length == 0 || maxPriority >= snapshot.priorities[0]) return all;
            var start = 0;
            while (start < all.length && snapshot.priorities[start] > maxPriority) start++;
            if (start == 0) return all;
            if (start == all.length) return EMPTY_HOOKERS;
            var result = new XposedInterface.Hooker[all.length - start];
            System.arraycopy(all, start, result, 0, result.length);
            return result;
        }
    }

    private static HookList hookList(Executable executable) {
        return sHookLists.computeIfAbsent(executable, ignored -> new HookList());
    }

    private static XposedInterface.Hooker[] hookersOf(Executable executable, int maxPriority) {
        var list = sHookLists.get(executable);
        return list == null ? EMPTY_HOOKERS : list.hookers(maxPriority);
    }

    private static final Method getCause;

    static {
        Method tmp;
        try {
            tmp = InvocationTargetException.class.getMethod("getCause");
        } catch (Throwable e) {
            tmp = null;
        }
        getCause = tmp;
    }

    public static void log(String text) {
        Log.i(TAG, text);
    }

    public static void log(Throwable t) {
        String logStr = Log.getStackTraceString(t);
        Log.e(TAG, logStr);
    }

    public static class NativeHooker<T extends Executable> {
        // All per-method state is resolved once; the callback path only reads fields.
        private final T method;
        private final Class<?> returnType;
        private final boolean isStatic;
        private final boolean isConstructor;
        private final HookList hookList;

        @SuppressWarnings("unchecked")
        private NativeHooker(Executable method) {
            this.method = (T) method;
            this.returnType = returnTypeOf(method);
            this.isStatic = Modifier.isStatic(method.getModifiers());
            this.isConstructor = method instanceof Constructor;
            // The HookList instance is stable for the lifetime of the method; caching it avoids a
            // ConcurrentHashMap lookup on every interception.
            this.hookList = hookList(method);
        }

        public Object callback(Object[] rawArgs) throws Throwable {
            Object thisObject;
            Object[] args;
            if (isStatic) {
                thisObject = null;
                args = rawArgs;
            } else {
                thisObject = rawArgs[0];
                args = new Object[rawArgs.length - 1];
                System.arraycopy(rawArgs, 1, args, 0, args.length);
            }

            var hookers = hookList.hookers(XposedInterface.PRIORITY_HIGHEST);
            if (hookers.length == 0) {
                return invokeOriginal(method, thisObject, args, isConstructor);
            }

            var chain = new ChainImpl<>(method, returnType, hookers, thisObject, args, false, isConstructor);
            try {
                return chain.proceed();
            } finally {
                chain.close();
            }
        }
    }

    private static Class<?> returnTypeOf(Executable executable) {
        if (!(executable instanceof Method method)) {
            return null;
        }
        var returnType = method.getReturnType();
        return returnType.isPrimitive() ? null : returnType;
    }

    private static Object checkReturnType(Object result, Class<?> returnType) {
        // Class.isInstance is an intrinsic; avoids a JNI trip per hook result.
        if (returnType != null && !returnType.isInstance(result)) {
            throw new ClassCastException(CAST_ERROR);
        }
        return result;
    }

    private static Object unwrapInvocationTarget(InvocationTargetException e) throws Throwable {
        if (getCause == null) {
            throw e;
        }
        Throwable cause = (Throwable) HookBridge.invokeOriginalMethod(getCause, e, EMPTY_ARRAY, false);
        if (cause != null) {
            throw cause;
        }
        throw e;
    }

    private static <T extends Executable> Object invokeOriginal(
            T method,
            Object thisObject,
            Object[] args,
            boolean isConstructor
    ) throws Throwable {
        try {
            return HookBridge.invokeOriginalMethod(method, thisObject, args, isConstructor);
        } catch (InvocationTargetException e) {
            return unwrapInvocationTarget(e);
        }
    }

    static class ChainImpl<T extends Executable> implements XposedInterface.Chain {
        // Thread identity is enough to enforce the same-thread rule; avoids a JNI gettid() call
        // on every interception.
        private final Thread thread = Thread.currentThread();
        private final T executable;
        private final Class<?> returnType;
        private final XposedInterface.Hooker[] hookers;
        private final boolean special;
        private final boolean isConstructor;
        private int index = -1;
        private boolean active = true;
        private Object thisObject;
        private Object[] args;
        private Object[] argsListViewSource;
        private List<Object> argsListView;

        ChainImpl(T executable, Class<?> returnType, XposedInterface.Hooker[] hookers,
                  Object thisObject, Object[] args, boolean special, boolean isConstructor) {
            this.executable = executable;
            this.returnType = returnType;
            this.hookers = hookers == null ? EMPTY_HOOKERS : hookers;
            this.thisObject = thisObject;
            this.args = args == null ? EMPTY_ARRAY : args;
            this.special = special;
            this.isConstructor = isConstructor;
        }

        void close() {
            active = false;
        }

        private void checkActive() {
            if (Thread.currentThread() != thread) {
                throw new IllegalStateException("Chain must be accessed in the same thread as the hooked method");
            }
            if (!active) {
                throw new IllegalStateException("Chain cannot be used after the interception ends");
            }
        }

        @NonNull
        @Override
        public Executable getExecutable() {
            return executable;
        }

        @Override
        public Object getThisObject() {
            return thisObject;
        }

        @NonNull
        @Override
        public List<Object> getArgs() {
            // Most hookers read args repeatedly; reuse the view until proceed(args) swaps them.
            var currentArgs = args;
            if (argsListView == null || argsListViewSource != currentArgs) {
                argsListView = Collections.unmodifiableList(Arrays.asList(currentArgs));
                argsListViewSource = currentArgs;
            }
            return argsListView;
        }

        @Override
        public Object getArg(int index) throws IndexOutOfBoundsException, ClassCastException {
            return args[index];
        }

        @Override
        public Object proceed() throws Throwable {
            checkActive();
            var next = ++index;
            try {
                if (next != hookers.length) {
                    Object result = hookers[next].intercept(this);
                    return checkReturnType(result, returnType);
                }
                if (special) {
                    try {
                        return checkReturnType(HookBridge.invokeSpecialMethod(executable, null, thisObject, args), returnType);
                    } catch (InvocationTargetException e) {
                        return unwrapInvocationTarget(e);
                    }
                }
                return checkReturnType(invokeOriginal(executable, thisObject, args, isConstructor), returnType);
            } finally {
                index--;
            }
        }

        @Override
        public Object proceed(@NonNull Object[] args) throws Throwable {
            var oldArgs = this.args;
            this.args = args;
            try {
                return proceed();
            } finally {
                this.args = oldArgs;
            }
        }

        @Override
        public Object proceedWith(@NonNull Object thisObject) throws Throwable {
            var oldThisObject = this.thisObject;
            this.thisObject = thisObject;
            try {
                return proceed();
            } finally {
                this.thisObject = oldThisObject;
            }
        }

        @Override
        public Object proceedWith(@NonNull Object thisObject, @NonNull Object[] args) throws Throwable {
            var oldThisObject = this.thisObject;
            var oldArgs = this.args;
            this.thisObject = thisObject;
            this.args = args;
            try {
                return proceed();
            } finally {
                this.thisObject = oldThisObject;
                this.args = oldArgs;
            }
        }
    }

    static class ProtectiveChain implements XposedInterface.Chain {
        private final XposedInterface.Chain base;
        private boolean proceeded;
        private Object result;
        private Throwable throwable;

        ProtectiveChain(XposedInterface.Chain base) {
            this.base = base;
        }

        @NonNull
        @Override
        public Executable getExecutable() {
            return base.getExecutable();
        }

        @Override
        public Object getThisObject() {
            return base.getThisObject();
        }

        @NonNull
        @Override
        public List<Object> getArgs() {
            return base.getArgs();
        }

        @Override
        public Object getArg(int index) throws IndexOutOfBoundsException, ClassCastException {
            return base.getArg(index);
        }

        @Override
        public Object proceed() throws Throwable {
            proceeded = true;
            try {
                result = base.proceed();
                return result;
            } catch (Throwable t) {
                throwable = t;
                throw t;
            }
        }

        @Override
        public Object proceed(@NonNull Object[] args) throws Throwable {
            proceeded = true;
            try {
                result = base.proceed(args);
                return result;
            } catch (Throwable t) {
                throwable = t;
                throw t;
            }
        }

        @Override
        public Object proceedWith(@NonNull Object thisObject) throws Throwable {
            proceeded = true;
            try {
                result = base.proceedWith(thisObject);
                return result;
            } catch (Throwable t) {
                throwable = t;
                throw t;
            }
        }

        @Override
        public Object proceedWith(@NonNull Object thisObject, @NonNull Object[] args) throws Throwable {
            proceeded = true;
            try {
                result = base.proceedWith(thisObject, args);
                return result;
            } catch (Throwable t) {
                throwable = t;
                throw t;
            }
        }
    }

    static class ProtectiveHooker implements XposedInterface.Hooker {
        private final XposedInterface context;
        private final XposedInterface.Hooker hooker;

        ProtectiveHooker(XposedInterface context, XposedInterface.Hooker hooker) {
            this.context = context;
            this.hooker = hooker;
        }

        @Override
        public Object intercept(@NonNull XposedInterface.Chain chain) throws Throwable {
            var protectiveChain = new ProtectiveChain(chain);
            try {
                return hooker.intercept(protectiveChain);
            } catch (Throwable t) {
                if (protectiveChain.throwable == t) {
                    throw t;
                }
                context.log(Log.WARN, "ProtectiveHooker", "Exception in hooker", t);
                if (!protectiveChain.proceeded) {
                    return chain.proceed();
                }
                if (protectiveChain.throwable != null) {
                    throw protectiveChain.throwable;
                }
                return protectiveChain.result;
            }
        }
    }

    static class HookBuilderImpl implements XposedInterface.HookBuilder {
        private final XposedInterface context;
        private final Executable executable;
        private final XposedInterface.ExceptionMode defaultExceptionMode;
        private int priority = XposedInterface.PRIORITY_DEFAULT;
        private XposedInterface.ExceptionMode exceptionMode = XposedInterface.ExceptionMode.DEFAULT;
        private String id = null;

        HookBuilderImpl(XposedInterface context, Executable executable,
                        XposedInterface.ExceptionMode defaultExceptionMode) {
            this.context = context;
            this.executable = executable;
            this.defaultExceptionMode = defaultExceptionMode;
        }

        @Override
        public XposedInterface.HookBuilder setPriority(int priority) {
            this.priority = priority;
            return this;
        }

        @Override
        public XposedInterface.HookBuilder setExceptionMode(@NonNull XposedInterface.ExceptionMode mode) {
            this.exceptionMode = mode;
            return this;
        }

        @NonNull
        @Override
        public XposedInterface.HookBuilder setId(@Nullable String id) {
            this.id = id;
            return this;
        }

        @NonNull
        @Override
        public XposedInterface.HookHandle intercept(@NonNull XposedInterface.Hooker hooker) {
            Objects.requireNonNull(hooker, "hooker should not be null!");
            var mode = resolveExceptionMode(exceptionMode, defaultExceptionMode);
            if (mode == XposedInterface.ExceptionMode.PROTECTIVE) {
                hooker = new ProtectiveHooker(context, hooker);
            }
            if (id == null) {
                return doHook(context, executable, priority, mode, null, hooker);
            }
            return doHookWithId(context, executable, priority, mode, id, hooker);
        }
    }

    static class HookHandleImpl implements XposedInterface.HookHandle {
        private final XposedInterface context;
        private final Executable executable;
        private final int priority;
        private final XposedInterface.ExceptionMode exceptionMode;
        private final String id;
        private final XposedInterface.Hooker hooker;
        private volatile boolean valid = true;

        HookHandleImpl(XposedInterface context, Executable executable, int priority,
                       XposedInterface.ExceptionMode exceptionMode, String id,
                       XposedInterface.Hooker hooker) {
            this.context = context;
            this.executable = executable;
            this.priority = priority;
            this.exceptionMode = exceptionMode;
            this.id = id;
            this.hooker = hooker;
        }

        boolean isValid() {
            return valid;
        }

        @NonNull
        @Override
        public Executable getExecutable() {
            return executable;
        }

        @Nullable
        @Override
        public String getId() {
            return id;
        }

        @Override
        public void unhook() {
            synchronized (executable) {
                if (!valid) return;
                valid = false;
                HookBridge.unhookMethod(executable, hooker);
                hookList(executable).remove(hooker);
                untrackHandle(context, this);
                if (id != null) {
                    var byId = hooksOf(context).get(executable);
                    if (byId != null) {
                        byId.remove(id, this);
                    }
                }
            }
        }

        @NonNull
        @Override
        public XposedInterface.HookHandle replaceHook(@NonNull XposedInterface.Hooker newHooker) {
            Objects.requireNonNull(newHooker, "hooker should not be null!");
            synchronized (executable) {
                if (!valid) {
                    throw new IllegalStateException("This hook handle is no longer valid");
                }
                var oldHooker = hooker;
                if (exceptionMode == XposedInterface.ExceptionMode.PROTECTIVE) {
                    newHooker = new ProtectiveHooker(context, newHooker);
                }
                HookBridge.unhookMethod(executable, oldHooker);
                hookList(executable).remove(oldHooker);
                if (!HookBridge.hookMethod(executable, NativeHooker.class, priority, newHooker)) {
                    // Restore the previous hook so this handle stays usable.
                    if (HookBridge.hookMethod(executable, NativeHooker.class, priority, oldHooker)) {
                        hookList(executable).add(priority, oldHooker);
                    }
                    throw new HookFailedError("Cannot replace hook " + executable);
                }
                hookList(executable).add(priority, newHooker);
                valid = false;
                untrackHandle(context, this);
                var newHandle = new HookHandleImpl(context, executable, priority, exceptionMode, id, newHooker);
                trackHandle(context, newHandle);
                if (id != null) {
                    hooksOf(context).computeIfAbsent(executable, k -> new ConcurrentHashMap<>()).put(id, newHandle);
                }
                return newHandle;
            }
        }
    }

    static class BaseInvoker<T extends Executable> {
        final T executable;
        private final Class<?> returnType;
        private final boolean isConstructor;
        protected Object target;

        BaseInvoker(T executable) {
            this.executable = executable;
            this.returnType = returnTypeOf(executable);
            this.isConstructor = executable instanceof Constructor;
            this.target = XposedInterface.Invoker.Type.Chain.FULL;
            executable.setAccessible(true);
        }

        public Object invoke(Object thisObject, Object... args) throws InvocationTargetException, IllegalArgumentException, IllegalAccessException {
            var type = (XposedInterface.Invoker.Type) target;
            Objects.requireNonNull(type);
            if (type instanceof XposedInterface.Invoker.Type.Origin) {
                return HookBridge.invokeOriginalMethod(executable, thisObject, args, isConstructor);
            }
            if (!(type instanceof XposedInterface.Invoker.Type.Chain chainType)) {
                throw new IllegalStateException("Unknown invoker type");
            }
            return invokeChain(thisObject, args, chainType.maxPriority(), false);
        }

        public Object invokeSpecial(@NonNull Object thisObject, Object... args) throws InvocationTargetException, IllegalArgumentException, IllegalAccessException {
            if (Modifier.isStatic(executable.getModifiers())) {
                throw new IllegalArgumentException("Cannot invoke special on static method: " + executable);
            }
            var type = (XposedInterface.Invoker.Type) target;
            Objects.requireNonNull(type);
            if (type instanceof XposedInterface.Invoker.Type.Origin) {
                try {
                    return HookBridge.invokeSpecialMethod(executable, null, thisObject, args);
                } catch (InstantiationException e) {
                    throw new InstantiationError(e.getMessage());
                }
            }
            if (!(type instanceof XposedInterface.Invoker.Type.Chain chainType)) {
                throw new IllegalStateException("Unknown invoker type");
            }
            return invokeChain(thisObject, args, chainType.maxPriority(), true);
        }

        Object invokeChain(Object thisObject, Object[] args, int maxPriority, boolean special)
                throws InvocationTargetException, IllegalAccessException {
            var hookers = hookersOf(executable, maxPriority);
            var chain = new ChainImpl<>(executable, returnType, hookers, thisObject, args, special, isConstructor);
            try {
                return chain.proceed();
            } catch (Error | RuntimeException | InvocationTargetException | IllegalAccessException e) {
                throw e;
            } catch (Throwable t) {
                throw new InvocationTargetException(t);
            } finally {
                chain.close();
            }
        }

        void setInvokerType(@NonNull XposedInterface.Invoker.Type type) {
            target = type;
        }
    }

    static class MethodInvokerImpl extends BaseInvoker<Method> implements XposedInterface.Invoker<MethodInvokerImpl, Method> {
        MethodInvokerImpl(Method executable) {
            super(executable);
        }

        @Override
        public MethodInvokerImpl setType(@NonNull XposedInterface.Invoker.Type type) {
            setInvokerType(type);
            return this;
        }
    }

    static class CtorInvokerImpl<T> extends BaseInvoker<Constructor<T>> implements XposedInterface.CtorInvoker<T> {
        CtorInvokerImpl(Constructor<T> executable) {
            super(executable);
        }

        @NonNull
        @Override
        public T newInstance(Object... args) throws InvocationTargetException, IllegalArgumentException, IllegalAccessException, InstantiationException {
            var instance = HookBridge.allocateObject(executable.getDeclaringClass());
            invoke(instance, args);
            return instance;
        }

        @NonNull
        @Override
        public <U> U newInstanceSpecial(@NonNull Class<U> subClass, Object... args) throws InvocationTargetException, IllegalArgumentException, IllegalAccessException, InstantiationException {
            var type = (XposedInterface.Invoker.Type) super.target;
            Objects.requireNonNull(type);
            if (type instanceof XposedInterface.Invoker.Type.Origin) {
                return (U) HookBridge.invokeSpecialMethod(executable, subClass, null, args);
            }
            if (!(type instanceof XposedInterface.Invoker.Type.Chain chainType)) {
                throw new IllegalStateException("Unknown invoker type");
            }
            var instance = HookBridge.allocateSpecialReceiver(executable, subClass);
            invokeChain(instance, args, chainType.maxPriority(), true);
            return instance;
        }

        @Override
        public XposedInterface.CtorInvoker<T> setType(@NonNull XposedInterface.Invoker.Type type) {
            setInvokerType(type);
            return this;
        }
    }

    private static XposedInterface.ExceptionMode resolveExceptionMode(
            XposedInterface.ExceptionMode mode,
            XposedInterface.ExceptionMode defaultMode
    ) {
        if (mode == XposedInterface.ExceptionMode.PROTECTIVE) {
            return XposedInterface.ExceptionMode.PROTECTIVE;
        }
        if (mode == XposedInterface.ExceptionMode.DEFAULT
                && defaultMode == XposedInterface.ExceptionMode.PROTECTIVE) {
            return XposedInterface.ExceptionMode.PROTECTIVE;
        }
        return XposedInterface.ExceptionMode.PASSTHROUGH;
    }

    private static void validateHookTarget(Executable hookMethod) {
        if (Modifier.isAbstract(hookMethod.getModifiers())) {
            throw new IllegalArgumentException("Cannot hook abstract methods: " + hookMethod);
        } else if (hookMethod.getDeclaringClass().getClassLoader() == LSPosedContext.class.getClassLoader()) {
            throw new IllegalArgumentException("Do not allow hooking inner methods");
        } else if (hookMethod.getDeclaringClass() == Method.class && hookMethod.getName().equals("invoke")) {
            throw new IllegalArgumentException("Cannot hook Method.invoke");
        }
    }

    private static ConcurrentHashMap<Executable, ConcurrentHashMap<String, HookHandleImpl>> hooksOf(
            XposedInterface context
    ) {
        synchronized (sHooksById) {
            return sHooksById.computeIfAbsent(context, k -> new ConcurrentHashMap<>());
        }
    }

    private static void trackHandle(XposedInterface context, HookHandleImpl handle) {
        if (context == null) return;
        synchronized (sLiveHandles) {
            sLiveHandles.computeIfAbsent(context, k -> ConcurrentHashMap.newKeySet()).add(handle);
        }
    }

    private static void untrackHandle(XposedInterface context, HookHandleImpl handle) {
        if (context == null) return;
        synchronized (sLiveHandles) {
            var handles = sLiveHandles.get(context);
            if (handles != null) handles.remove(handle);
        }
    }

    /**
     * Returns a snapshot of the live hook handles created by the given module generation.
     */
    static List<XposedInterface.HookHandle> captureHandles(XposedInterface context) {
        synchronized (sLiveHandles) {
            var handles = sLiveHandles.get(context);
            if (handles == null) return Collections.emptyList();
            return new ArrayList<XposedInterface.HookHandle>(handles);
        }
    }

    private static void checkNotFrozen(XposedInterface context) {
        if (context instanceof LSPosedContext ctx && ctx.isFrozen()) {
            throw new IllegalStateException("The module generation is frozen for hot reload");
        }
    }

    static HookHandleImpl doHook(
            XposedInterface context,
            Executable hookMethod,
            int priority,
            XposedInterface.ExceptionMode mode,
            String id,
            XposedInterface.Hooker hooker
    ) {
        if (hooker == null) {
            throw new IllegalArgumentException("hooker should not be null!");
        }
        validateHookTarget(hookMethod);
        if (HookBridge.hookMethod(hookMethod, LSPosedBridge.NativeHooker.class, priority, hooker)) {
            hookList(hookMethod).add(priority, hooker);
            var handle = new HookHandleImpl(context, hookMethod, priority, mode, id, hooker);
            trackHandle(context, handle);
            return handle;
        }
        throw new HookFailedError("Cannot hook " + hookMethod);
    }

    private static HookHandleImpl doHookWithId(
            XposedInterface context,
            Executable hookMethod,
            int priority,
            XposedInterface.ExceptionMode mode,
            String id,
            XposedInterface.Hooker hooker
    ) {
        synchronized (hookMethod) {
            if (hooker == null) {
                throw new IllegalArgumentException("hooker should not be null!");
            }
            validateHookTarget(hookMethod);
            var byId = hooksOf(context).computeIfAbsent(hookMethod, k -> new ConcurrentHashMap<>());
            var old = byId.get(id);
            if (old != null && old.isValid()) {
                // unhook() removes itself from byId and marks the old handle invalid.
                old.unhook();
            }
            if (!HookBridge.hookMethod(hookMethod, LSPosedBridge.NativeHooker.class, priority, hooker)) {
                throw new HookFailedError("Cannot hook " + hookMethod);
            }
            hookList(hookMethod).add(priority, hooker);
            var handle = new HookHandleImpl(context, hookMethod, priority, mode, id, hooker);
            trackHandle(context, handle);
            byId.put(id, handle);
            return handle;
        }
    }

    public static XposedInterface.HookHandle doHook(
            Executable hookMethod,
            int priority,
            XposedInterface.Hooker hooker
    ) {
        if (hooker == null) {
            throw new IllegalArgumentException("hooker should not be null!");
        }
        return doHook(null, hookMethod, priority, XposedInterface.ExceptionMode.PASSTHROUGH, null, hooker);
    }

    /**
     * Keeps the Java-side hooker cache in sync when a hooker is removed without going through a
     * {@link XposedInterface.HookHandle} (the deprecated legacy unhook path).
     */
    public static void doUnhook(Executable hookMethod, XposedInterface.Hooker hooker) {
        hookList(hookMethod).remove(hooker);
    }

    public static XposedInterface.HookBuilder newHookBuilder(
            XposedInterface context,
            Executable executable,
            XposedInterface.ExceptionMode defaultExceptionMode
    ) {
        Objects.requireNonNull(executable, "origin must not be null");
        checkNotFrozen(context);
        return new HookBuilderImpl(context, executable, defaultExceptionMode);
    }

    public static XposedInterface.HookBuilder newClassInitializerHookBuilder(
            XposedInterface context,
            Class<?> clazz,
            XposedInterface.ExceptionMode defaultExceptionMode
    ) {
        Objects.requireNonNull(clazz, "origin must not be null");
        checkNotFrozen(context);
        if (clazz.getClassLoader() == LSPosedContext.class.getClassLoader()) {
            throw new IllegalArgumentException("Do not allow hooking inner classes");
        }
        synchronized (clazz) {
            Method classInitializer = HookBridge.findClassInitializer(clazz);
            if (classInitializer == null) {
                throw new IllegalArgumentException("Cannot find class initializer for " + clazz);
            }
            return new HookBuilderImpl(context, classInitializer, defaultExceptionMode);
        }
    }

    public static boolean doDeoptimize(@NonNull Executable executable) {
        Objects.requireNonNull(executable, "executable must not be null");
        if (Modifier.isAbstract(executable.getModifiers())) {
            throw new IllegalArgumentException("Cannot deoptimize abstract methods: " + executable);
        } else if (Proxy.isProxyClass(executable.getDeclaringClass())) {
            throw new IllegalArgumentException("Cannot deoptimize methods from proxy class: " + executable);
        }
        return HookBridge.deoptimizeMethod(executable);
    }

    public static XposedInterface.Invoker<?, Method> newInvoker(@NonNull Method method) {
        Objects.requireNonNull(method, "method must not be null");
        return new MethodInvokerImpl(method);
    }

    public static <T> XposedInterface.CtorInvoker<T> newInvoker(@NonNull Constructor<T> constructor) {
        Objects.requireNonNull(constructor, "constructor must not be null");
        return new CtorInvokerImpl<>(constructor);
    }
}
