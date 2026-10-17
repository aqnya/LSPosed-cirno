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
 * Copyright (C) 2022 LSPosed Contributors
 */

#include "lsplant.hpp"
#include <parallel_hashmap/phmap.h>
#include <array>
#include <cstdio>
#include <limits>
#include <memory>
#include <shared_mutex>
#include <mutex>
#include <set>
#include <unistd.h>

using namespace lsplant;

namespace {
struct InvokeCache {
    jclass object_class;
    jclass class_class;
    jclass executable_class;
    jclass method_class;
    jclass constructor_class;

    jclass boolean_class;
    jclass byte_class;
    jclass character_class;
    jclass double_class;
    jclass float_class;
    jclass integer_class;
    jclass long_class;
    jclass number_class;
    jclass short_class;
    jclass void_class;

    jclass void_type;
    jclass boolean_type;
    jclass byte_type;
    jclass char_type;
    jclass double_type;
    jclass float_type;
    jclass int_type;
    jclass long_type;
    jclass short_type;

    jmethodID executable_get_declaring_class;
    jmethodID executable_get_parameter_types;
    jmethodID class_get_type_name;
    jmethodID method_get_return_type;
    jmethodID method_invoke;
    jmethodID constructor_new_instance;

    jmethodID boolean_value;
    jmethodID byte_value;
    jmethodID char_value;
    jmethodID double_value;
    jmethodID float_value;
    jmethodID int_value;
    jmethodID long_value;
    jmethodID short_value;

    jmethodID boolean_value_of;
    jmethodID byte_value_of;
    jmethodID char_value_of;
    jmethodID double_value_of;
    jmethodID float_value_of;
    jmethodID int_value_of;
    jmethodID long_value_of;
    jmethodID short_value_of;
};

jclass NewGlobalClassRef(JNIEnv *env, const char *name) {
    auto local = env->FindClass(name);
    if (local == nullptr) return nullptr;
    auto global = (jclass) env->NewGlobalRef(local);
    env->DeleteLocalRef(local);
    return global;
}

template <typename T>
T NewGlobalRef(JNIEnv *env, T local) {
    if (local == nullptr) return nullptr;
    auto global = (T) env->NewGlobalRef(local);
    env->DeleteLocalRef(local);
    return global;
}

jclass GetPrimitiveType(JNIEnv *env, jclass wrapper_class) {
    auto type_field = env->GetStaticFieldID(wrapper_class, "TYPE", "Ljava/lang/Class;");
    if (type_field == nullptr) return nullptr;
    return NewGlobalRef(env, (jclass) env->GetStaticObjectField(wrapper_class, type_field));
}

InvokeCache &GetInvokeCache(JNIEnv *env) {
    static auto cache = [env] {
        InvokeCache c {};
        c.object_class = NewGlobalClassRef(env, "java/lang/Object");
        c.class_class = NewGlobalClassRef(env, "java/lang/Class");
        c.executable_class = NewGlobalClassRef(env, "java/lang/reflect/Executable");
        c.method_class = NewGlobalClassRef(env, "java/lang/reflect/Method");
        c.constructor_class = NewGlobalClassRef(env, "java/lang/reflect/Constructor");
        c.boolean_class = NewGlobalClassRef(env, "java/lang/Boolean");
        c.byte_class = NewGlobalClassRef(env, "java/lang/Byte");
        c.character_class = NewGlobalClassRef(env, "java/lang/Character");
        c.double_class = NewGlobalClassRef(env, "java/lang/Double");
        c.float_class = NewGlobalClassRef(env, "java/lang/Float");
        c.integer_class = NewGlobalClassRef(env, "java/lang/Integer");
        c.long_class = NewGlobalClassRef(env, "java/lang/Long");
        c.number_class = NewGlobalClassRef(env, "java/lang/Number");
        c.short_class = NewGlobalClassRef(env, "java/lang/Short");
        c.void_class = NewGlobalClassRef(env, "java/lang/Void");

        c.void_type = GetPrimitiveType(env, c.void_class);
        c.boolean_type = GetPrimitiveType(env, c.boolean_class);
        c.byte_type = GetPrimitiveType(env, c.byte_class);
        c.char_type = GetPrimitiveType(env, c.character_class);
        c.double_type = GetPrimitiveType(env, c.double_class);
        c.float_type = GetPrimitiveType(env, c.float_class);
        c.int_type = GetPrimitiveType(env, c.integer_class);
        c.long_type = GetPrimitiveType(env, c.long_class);
        c.short_type = GetPrimitiveType(env, c.short_class);

        c.executable_get_declaring_class = env->GetMethodID(c.executable_class, "getDeclaringClass",
                                                             "()Ljava/lang/Class;");
        c.executable_get_parameter_types = env->GetMethodID(c.executable_class, "getParameterTypes",
                                                             "()[Ljava/lang/Class;");
        c.class_get_type_name = env->GetMethodID(c.class_class, "getTypeName", "()Ljava/lang/String;");
        c.method_get_return_type = env->GetMethodID(c.method_class, "getReturnType", "()Ljava/lang/Class;");
        c.method_invoke = env->GetMethodID(c.method_class, "invoke",
                                           "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;");
        c.constructor_new_instance = env->GetMethodID(c.constructor_class, "newInstance",
                                                      "([Ljava/lang/Object;)Ljava/lang/Object;");
        c.boolean_value = env->GetMethodID(c.boolean_class, "booleanValue", "()Z");
        c.byte_value = env->GetMethodID(c.byte_class, "byteValue", "()B");
        c.char_value = env->GetMethodID(c.character_class, "charValue", "()C");
        c.double_value = env->GetMethodID(c.number_class, "doubleValue", "()D");
        c.float_value = env->GetMethodID(c.number_class, "floatValue", "()F");
        c.int_value = env->GetMethodID(c.number_class, "intValue", "()I");
        c.long_value = env->GetMethodID(c.number_class, "longValue", "()J");
        c.short_value = env->GetMethodID(c.number_class, "shortValue", "()S");
        c.boolean_value_of = env->GetStaticMethodID(c.boolean_class, "valueOf", "(Z)Ljava/lang/Boolean;");
        c.byte_value_of = env->GetStaticMethodID(c.byte_class, "valueOf", "(B)Ljava/lang/Byte;");
        c.char_value_of = env->GetStaticMethodID(c.character_class, "valueOf", "(C)Ljava/lang/Character;");
        c.double_value_of = env->GetStaticMethodID(c.double_class, "valueOf", "(D)Ljava/lang/Double;");
        c.float_value_of = env->GetStaticMethodID(c.float_class, "valueOf", "(F)Ljava/lang/Float;");
        c.int_value_of = env->GetStaticMethodID(c.integer_class, "valueOf", "(I)Ljava/lang/Integer;");
        c.long_value_of = env->GetStaticMethodID(c.long_class, "valueOf", "(J)Ljava/lang/Long;");
        c.short_value_of = env->GetStaticMethodID(c.short_class, "valueOf", "(S)Ljava/lang/Short;");
        return c;
    }();
    return cache;
}

void Throw(JNIEnv *env, const char *type, const char *message) {
    ScopedLocalRef<jclass> clazz(env, env->FindClass(type));
    if (clazz) env->ThrowNew(clazz.get(), message);
}

void ThrowArgumentTypeMismatch(JNIEnv *env) {
    Throw(env, "java/lang/IllegalArgumentException", "argument type mismatch");
}

void ThrowUnexpectedReceiverException(JNIEnv *env, InvokeCache &cache, jclass expected, jclass actual) {
    ScopedLocalRef<jstring> expected_type_jstr(env, reinterpret_cast<jstring>(
            env->CallObjectMethod(expected, cache.class_get_type_name)));
    if (!expected_type_jstr || env->ExceptionCheck()) return;

    auto expected_type = env->GetStringUTFChars(expected_type_jstr.get(), nullptr);
    if (expected_type == nullptr || env->ExceptionCheck()) return;

    ScopedLocalRef<jstring> actual_type_jstr(env, reinterpret_cast<jstring>(
            env->CallObjectMethod(actual, cache.class_get_type_name)));
    if (!actual_type_jstr || env->ExceptionCheck()) {
        env->ReleaseStringUTFChars(expected_type_jstr.get(), expected_type);
        return;
    }

    auto actual_type = env->GetStringUTFChars(actual_type_jstr.get(), nullptr);
    if (actual_type == nullptr || env->ExceptionCheck()) {
        env->ReleaseStringUTFChars(expected_type_jstr.get(), expected_type);
        return;
    }

    std::array<char, 1024> msg {};
    std::snprintf(msg.data(), msg.size(), "Expected receiver of type %s, but got %s",
                  expected_type, actual_type);
    env->ReleaseStringUTFChars(actual_type_jstr.get(), actual_type);
    env->ReleaseStringUTFChars(expected_type_jstr.get(), expected_type);

    Throw(env, "java/lang/IllegalArgumentException", msg.data());
}

void ThrowUnexpectedReceiverException(JNIEnv *env, InvokeCache &cache, jclass expected, jobject actual) {
    if (actual == nullptr) {
        Throw(env, "java/lang/IllegalArgumentException", "object is not an instance of declaring class");
        return;
    }

    ScopedLocalRef<jclass> actual_class(env, env->GetObjectClass(actual));
    if (!actual_class || env->ExceptionCheck()) {
        Throw(env, "java/lang/IllegalArgumentException", "object is not an instance of declaring class");
        return;
    }

    ThrowUnexpectedReceiverException(env, cache, expected, actual_class.get());
}

jchar TypeShorty(JNIEnv *env, InvokeCache &cache, jclass type) {
    if (env->IsSameObject(type, cache.int_type)) return 'I';
    if (env->IsSameObject(type, cache.long_type)) return 'J';
    if (env->IsSameObject(type, cache.float_type)) return 'F';
    if (env->IsSameObject(type, cache.double_type)) return 'D';
    if (env->IsSameObject(type, cache.boolean_type)) return 'Z';
    if (env->IsSameObject(type, cache.byte_type)) return 'B';
    if (env->IsSameObject(type, cache.char_type)) return 'C';
    if (env->IsSameObject(type, cache.short_type)) return 'S';
    if (env->IsSameObject(type, cache.void_type)) return 'V';
    return 'L';
}

bool ThrowInvocationTargetException(JNIEnv *env) {
    ScopedLocalRef<jthrowable> exception(env, env->ExceptionOccurred());
    if (!exception) return false;

    env->ExceptionClear();
    ScopedLocalRef<jclass> clazz(env, env->FindClass("java/lang/reflect/InvocationTargetException"));
    if (!clazz) return true;

    auto constructor = env->GetMethodID(clazz.get(), "<init>", "(Ljava/lang/Throwable;)V");
    ScopedLocalRef<jobject> wrapped(env, env->NewObject(clazz.get(), constructor, exception.get()));
    if (wrapped) env->Throw((jthrowable) wrapped.get());
    return true;
}

bool UnboxPrimitiveArg(JNIEnv *env, InvokeCache &cache, jchar type, jobject element,
                       jvalue &value) {
    if (element == nullptr) {
        Throw(env, "java/lang/NullPointerException", "argument == null");
        return false;
    }
    auto is_boolean = env->IsInstanceOf(element, cache.boolean_class);
    auto is_byte = env->IsInstanceOf(element, cache.byte_class);
    auto is_char = env->IsInstanceOf(element, cache.character_class);
    auto is_double = env->IsInstanceOf(element, cache.double_class);
    auto is_float = env->IsInstanceOf(element, cache.float_class);
    auto is_int = env->IsInstanceOf(element, cache.integer_class);
    auto is_long = env->IsInstanceOf(element, cache.long_class);
    auto is_short = env->IsInstanceOf(element, cache.short_class);
    switch (type) {
        case 'Z':
            if (!is_boolean) {
                ThrowArgumentTypeMismatch(env);
                return false;
            }
            value.z = env->CallBooleanMethod(element, cache.boolean_value);
            return !env->ExceptionCheck();
        case 'B':
            if (!is_byte) {
                ThrowArgumentTypeMismatch(env);
                return false;
            }
            value.b = env->CallByteMethod(element, cache.byte_value);
            return !env->ExceptionCheck();
        case 'C':
            if (!is_char) {
                ThrowArgumentTypeMismatch(env);
                return false;
            }
            value.c = env->CallCharMethod(element, cache.char_value);
            return !env->ExceptionCheck();
        case 'S':
            if (is_byte) {
                value.s = env->CallByteMethod(element, cache.byte_value);
                return !env->ExceptionCheck();
            }
            if (is_short) {
                value.s = env->CallShortMethod(element, cache.short_value);
                return !env->ExceptionCheck();
            }
            ThrowArgumentTypeMismatch(env);
            return false;
        case 'I':
            if (is_char) {
                value.i = env->CallCharMethod(element, cache.char_value);
                return !env->ExceptionCheck();
            }
            if (is_byte || is_short || is_int) {
                value.i = env->CallIntMethod(element, cache.int_value);
                return !env->ExceptionCheck();
            }
            ThrowArgumentTypeMismatch(env);
            return false;
        case 'J':
            if (is_char) {
                value.j = env->CallCharMethod(element, cache.char_value);
                return !env->ExceptionCheck();
            }
            if (is_byte || is_short || is_int || is_long) {
                value.j = env->CallLongMethod(element, cache.long_value);
                return !env->ExceptionCheck();
            }
            ThrowArgumentTypeMismatch(env);
            return false;
        case 'F':
            if (is_char) {
                value.f = env->CallCharMethod(element, cache.char_value);
                return !env->ExceptionCheck();
            }
            if (is_byte || is_short || is_int || is_long || is_float) {
                value.f = env->CallFloatMethod(element, cache.float_value);
                return !env->ExceptionCheck();
            }
            ThrowArgumentTypeMismatch(env);
            return false;
        case 'D':
            if (is_char) {
                value.d = env->CallCharMethod(element, cache.char_value);
                return !env->ExceptionCheck();
            }
            if (is_byte || is_short || is_int || is_long || is_float || is_double) {
                value.d = env->CallDoubleMethod(element, cache.double_value);
                return !env->ExceptionCheck();
            }
            ThrowArgumentTypeMismatch(env);
            return false;
        default:
            ThrowArgumentTypeMismatch(env);
            return false;
    }
}

struct MappedArgs {
    JNIEnv *env;
    jsize length;
    jvalue *values;
    jboolean *local_refs;

    MappedArgs(JNIEnv *env, jsize length, jvalue *values, jboolean *local_refs)
        : env(env), length(length), values(values), local_refs(local_refs) {
        for (jsize i = 0; i != length; ++i) {
            this->local_refs[i] = JNI_FALSE;
        }
    }

    ~MappedArgs() {
        for (jsize i = 0; i != length; ++i) {
            if (local_refs[i] && values[i].l != nullptr) env->DeleteLocalRef(values[i].l);
        }
    }

    void KeepLocalRef(jsize index, jobject ref) {
        values[index].l = ref;
        local_refs[index] = JNI_TRUE;
    }
};

bool MapInvokeArgs(JNIEnv *env, InvokeCache &cache, jobjectArray parameter_types,
                   jobjectArray args, MappedArgs &mapped_args) {
    for (jsize i = 0; i != mapped_args.length; ++i) {
        ScopedLocalRef<jobject> element(env, args == nullptr ? nullptr : env->GetObjectArrayElement(args, i));
        if (env->ExceptionCheck()) return false;

        ScopedLocalRef<jclass> parameter_type(env, (jclass) env->GetObjectArrayElement(parameter_types, i));
        if (env->ExceptionCheck()) return false;

        auto type = TypeShorty(env, cache, parameter_type.get());
        if (type == 'L') {
            if (element && !env->IsInstanceOf(element.get(), parameter_type.get())) {
                ThrowArgumentTypeMismatch(env);
                return false;
            }
            mapped_args.KeepLocalRef(i, element.release());
        } else if (!UnboxPrimitiveArg(env, cache, type, element.get(), mapped_args.values[i])
                   || env->ExceptionCheck()) {
            return false;
        }
    }
    return true;
}

ScopedLocalRef<jobject> AllocateReceiver(JNIEnv *env, jclass cls) {
    return ScopedLocalRef<jobject>(env, env->AllocObject(cls));
}

ScopedLocalRef<jobject> AllocateSpecialReceiver(JNIEnv *env, InvokeCache &cache, jobject constructor,
                                                jclass alloc_class) {
    if (constructor == nullptr) {
        Throw(env, "java/lang/NullPointerException", "constructor == null");
        return ScopedLocalRef<jobject>(env);
    }
    if (alloc_class == nullptr) {
        Throw(env, "java/lang/NullPointerException", "subClass == null");
        return ScopedLocalRef<jobject>(env);
    }

    ScopedLocalRef<jclass> declaring_class(env, (jclass) env->CallObjectMethod(
            constructor, cache.executable_get_declaring_class));
    if (env->ExceptionCheck()) return ScopedLocalRef<jobject>(env);

    if (!env->IsAssignableFrom(alloc_class, declaring_class.get())) {
        ThrowUnexpectedReceiverException(env, cache, declaring_class.get(), alloc_class);
        return ScopedLocalRef<jobject>(env);
    }
    return AllocateReceiver(env, alloc_class);
}

jobject BoxPrimitiveResult(JNIEnv *env, InvokeCache &cache, jchar type, jvalue value) {
    switch (type) {
        case 'I':
            return env->CallStaticObjectMethod(cache.integer_class, cache.int_value_of, value.i);
        case 'D':
            return env->CallStaticObjectMethod(cache.double_class, cache.double_value_of, value.d);
        case 'J':
            return env->CallStaticObjectMethod(cache.long_class, cache.long_value_of, value.j);
        case 'F':
            return env->CallStaticObjectMethod(cache.float_class, cache.float_value_of, value.f);
        case 'S':
            return env->CallStaticObjectMethod(cache.short_class, cache.short_value_of, value.s);
        case 'B':
            return env->CallStaticObjectMethod(cache.byte_class, cache.byte_value_of, value.b);
        case 'C':
            return env->CallStaticObjectMethod(cache.character_class, cache.char_value_of, value.c);
        case 'Z':
            return env->CallStaticObjectMethod(cache.boolean_class, cache.boolean_value_of, value.z);
        default:
            return nullptr;
    }
}

jobject CallNonvirtualAndBox(JNIEnv *env, InvokeCache &cache, jchar return_type, jobject thiz,
                             jclass declaring_class, jmethodID target, jvalue *args) {
    jvalue result {};
    switch (return_type) {
        case 'I':
            result.i = env->CallNonvirtualIntMethodA(thiz, declaring_class, target, args);
            break;
        case 'D':
            result.d = env->CallNonvirtualDoubleMethodA(thiz, declaring_class, target, args);
            break;
        case 'J':
            result.j = env->CallNonvirtualLongMethodA(thiz, declaring_class, target, args);
            break;
        case 'F':
            result.f = env->CallNonvirtualFloatMethodA(thiz, declaring_class, target, args);
            break;
        case 'S':
            result.s = env->CallNonvirtualShortMethodA(thiz, declaring_class, target, args);
            break;
        case 'B':
            result.b = env->CallNonvirtualByteMethodA(thiz, declaring_class, target, args);
            break;
        case 'C':
            result.c = env->CallNonvirtualCharMethodA(thiz, declaring_class, target, args);
            break;
        case 'Z':
            result.z = env->CallNonvirtualBooleanMethodA(thiz, declaring_class, target, args);
            break;
        case 'L': {
            auto value = env->CallNonvirtualObjectMethodA(thiz, declaring_class, target, args);
            return ThrowInvocationTargetException(env) ? nullptr : value;
        }
        default:
        case 'V':
            env->CallNonvirtualVoidMethodA(thiz, declaring_class, target, args);
            ThrowInvocationTargetException(env);
            return nullptr;
    }

    if (ThrowInvocationTargetException(env)) return nullptr;
    return BoxPrimitiveResult(env, cache, return_type, result);
}

jobject InvokeSpecial(JNIEnv *env, jobject method, jclass alloc_class, jobject thiz, jobjectArray args) {
    auto &cache = GetInvokeCache(env);
    ScopedLocalRef<jclass> declaring_class(env, (jclass) env->CallObjectMethod(
            method, cache.executable_get_declaring_class));
    if (env->ExceptionCheck()) return nullptr;

    auto is_constructor = env->IsInstanceOf(method, cache.constructor_class);
    if (alloc_class != nullptr && !is_constructor) {
        Throw(env, "java/lang/IllegalArgumentException", "clazz is only supported for constructors");
        return nullptr;
    }

    ScopedLocalRef<jobject> allocated_receiver(env);
    if (thiz == nullptr && is_constructor && alloc_class != nullptr) {
        allocated_receiver = AllocateSpecialReceiver(env, cache, method, alloc_class);
        if (env->ExceptionCheck()) return nullptr;
        thiz = allocated_receiver.get();
    }
    if (thiz == nullptr) {
        Throw(env, "java/lang/NullPointerException", "null receiver");
        return nullptr;
    }
    if (!env->IsInstanceOf(thiz, declaring_class.get())) {
        ThrowUnexpectedReceiverException(env, cache, declaring_class.get(), thiz);
        return nullptr;
    }

    ScopedLocalRef<jobjectArray> parameter_types(env, (jobjectArray) env->CallObjectMethod(
            method, cache.executable_get_parameter_types));
    if (env->ExceptionCheck()) return nullptr;

    auto target = env->FromReflectedMethod(method);
    if (env->ExceptionCheck()) return nullptr;

    auto param_len = env->GetArrayLength(parameter_types.get());
    auto args_len = args == nullptr ? 0 : env->GetArrayLength(args);
    if (args_len != param_len) {
        Throw(env, "java/lang/IllegalArgumentException", "args.length != parameters.length");
        return nullptr;
    }

    jvalue *mapped_values = nullptr;
    jboolean *local_refs = nullptr;
    if (param_len != 0) {
        mapped_values = static_cast<jvalue *>(__builtin_alloca(sizeof(jvalue) * param_len));
        local_refs = static_cast<jboolean *>(__builtin_alloca(sizeof(jboolean) * param_len));
    }
    MappedArgs mapped_args(env, param_len, mapped_values, local_refs);
    if (!MapInvokeArgs(env, cache, parameter_types.get(), args, mapped_args)) {
        return nullptr;
    }

    jchar return_type = 'V';
    if (!is_constructor) {
        ScopedLocalRef<jclass> return_class(env, (jclass) env->CallObjectMethod(
                method, cache.method_get_return_type));
        if (env->ExceptionCheck()) return nullptr;
        return_type = TypeShorty(env, cache, return_class.get());
    }

    auto value = CallNonvirtualAndBox(env, cache, return_type, thiz, declaring_class.get(), target,
                                      mapped_args.values);
    if (return_type == 'V' && is_constructor && alloc_class != nullptr && !env->ExceptionCheck()) {
        return allocated_receiver.release();
    }
    return value;
}

jobject InvokeBackup(JNIEnv *env, InvokeCache &cache, jobject executable, jobject backup,
                     jobject thiz, jobjectArray args, bool is_constructor) {
    if (!is_constructor || thiz != nullptr) {
        return env->CallObjectMethod(backup, cache.method_invoke, thiz, args);
    }

    ScopedLocalRef<jclass> declaring_class(env, (jclass) env->CallObjectMethod(
            executable, cache.executable_get_declaring_class));
    if (env->ExceptionCheck()) return nullptr;

    auto receiver = AllocateReceiver(env, declaring_class.get());
    if (env->ExceptionCheck()) return nullptr;

    env->CallObjectMethod(backup, cache.method_invoke, receiver.get(), args);
    if (env->ExceptionCheck()) return nullptr;
    return receiver.release();
}

jobject NewInstance(JNIEnv *env, InvokeCache &cache, jobject constructor, jobjectArray args) {
    ScopedLocalRef<jobjectArray> empty_args(env);
    if (args == nullptr) {
        empty_args.reset(env->NewObjectArray(0, cache.object_class, nullptr));
        args = empty_args.get();
    }
    return env->CallObjectMethod(constructor, cache.constructor_new_instance, args);
}
}

extern "C" jobject lspd_lsplant_hook(JNIEnv *env, jobject method, jobject hooker,
                                       jobject callback) {
    return lsplant::Hook(env, method, hooker, callback);
}

extern "C" jboolean lspd_lsplant_deoptimize(JNIEnv *env, jobject method) {
    return lsplant::Deoptimize(env, method);
}

extern "C" jobject lspd_hook_bridge_invoke_original(JNIEnv *env, jobject method,
                                                     jobject receiver, jobjectArray args,
                                                     jboolean is_constructor, jobject backup) {
    auto &cache = GetInvokeCache(env);
    if (backup) {
        return InvokeBackup(env, cache, method, backup, receiver, args, is_constructor);
    }
    if (is_constructor) {
        if (receiver == nullptr) {
            return NewInstance(env, cache, method, args);
        }
        return InvokeSpecial(env, method, nullptr, receiver, args);
    }
    return env->CallObjectMethod(method, cache.method_invoke, receiver, args);
}

extern "C" jobject lspd_hook_bridge_allocate_special(JNIEnv *env, jobject constructor,
                                                      jclass clazz) {
    auto &cache = GetInvokeCache(env);
    return AllocateSpecialReceiver(env, cache, constructor, clazz).release();
}

extern "C" jobject lspd_hook_bridge_invoke_special(JNIEnv *env, jobject method, jclass clazz,
                                                    jobject receiver, jobjectArray args) {
    return InvokeSpecial(env, method, clazz, receiver, args);
}
