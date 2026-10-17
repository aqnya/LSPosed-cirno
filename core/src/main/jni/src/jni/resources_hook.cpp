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
 * Copyright (C) 2020 EdXposed Contributors
 * Copyright (C) 2021 - 2022 LSPosed Contributors
 */

#include <jni.h>
#include "dex_builder.h"
#include "framework/androidfw/resource_types.h"
#include "elf_util.h"
#include "native_util.h"
#include "config_bridge.h"

using namespace lsplant;

namespace {
    using TYPE_GET_ATTR_NAME_ID = int32_t (*)(void *, int);
    using TYPE_STRING_AT = char16_t *(*)(const void *, int32_t, size_t *);
    using TYPE_RESTART = void (*)(void *);
    using TYPE_NEXT = int32_t (*)(void *);

    TYPE_NEXT ResXMLParser_next = nullptr;
    TYPE_RESTART ResXMLParser_restart = nullptr;
    TYPE_GET_ATTR_NAME_ID ResXMLParser_getAttributeNameID = nullptr;

    std::string GetXResourcesClassName() {
        auto &obfs_map = ConfigBridge::GetInstance()->obfuscation_map();
        if (obfs_map.empty()) {
            LOGW("GetXResourcesClassName: obfuscation_map empty?????");
        }
        static auto name = lspd::JavaNameToSignature(
                obfs_map.at("android.content.res.XRes"))  // TODO: kill this hardcoded name
                .substr(1) + "ources";
        LOGD("{}", name.c_str());
        return name;
    }

    bool PrepareSymbols() {
        SandHook::ElfImg fw(kLibFwName);
        if (!fw.isValid()) {
            return false;
        };
        if (!(ResXMLParser_next = fw.getSymbAddress<TYPE_NEXT>(
                "_ZN7android12ResXMLParser4nextEv"))) {
            return false;
        }
        if (!(ResXMLParser_restart = fw.getSymbAddress<TYPE_RESTART>(
                "_ZN7android12ResXMLParser7restartEv"))) {
            return false;
        };
        if (!(ResXMLParser_getAttributeNameID = fw.getSymbAddress<TYPE_GET_ATTR_NAME_ID>(
                LP_SELECT("_ZNK7android12ResXMLParser18getAttributeNameIDEj",
                          "_ZNK7android12ResXMLParser18getAttributeNameIDEm")))) {
            return false;
        }
        return android::ResStringPool::setup(InitInfo{
                .art_symbol_resolver = [&](auto s) {
                    return fw.template getSymbAddress<>(s);
                }
        });
    }
}

extern "C" const char *lspd_resources_class_name(size_t *length) {
    static auto name = GetXResourcesClassName();
    if (length) *length = name.size();
    return name.c_str();
}

extern "C" jboolean lspd_resources_prepare_symbols() {
    return PrepareSymbols();
}

extern "C" jboolean lspd_resources_make_inheritable(JNIEnv *env, jclass clazz) {
    return lsplant::MakeClassInheritable(env, clazz);
}

extern "C" jobject lspd_resources_build_dummy(JNIEnv *env, jobject parent,
                                               jstring resource_super_class,
                                               jstring typed_array_super_class) {
    using namespace startop::dex;
    static auto in_memory_classloader = JNI_NewGlobalRef(env, JNI_FindClass(env,
                                                                            "dalvik/system/InMemoryDexClassLoader"));
    static jmethodID initMid = JNI_GetMethodID(env, in_memory_classloader, "<init>",
                                               "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V");
    DexBuilder dex_file;

    ClassBuilder xresource_builder{
            dex_file.MakeClass("xposed.dummy.XResourcesSuperClass")};
    xresource_builder.setSuperClass(TypeDescriptor::FromClassname(JUTFString(env, resource_super_class).get()));

    ClassBuilder xtypearray_builder{
            dex_file.MakeClass("xposed.dummy.XTypedArraySuperClass")};
    xtypearray_builder.setSuperClass(TypeDescriptor::FromClassname(JUTFString(env, typed_array_super_class).get()));

    slicer::MemView image{dex_file.CreateImage()};

    auto dex_buffer = env->NewDirectByteBuffer(const_cast<void *>(image.ptr()), image.size());
    return JNI_NewObject(env, in_memory_classloader, initMid, dex_buffer, parent).release();
}

extern "C" void lspd_resources_rewrite_xml(JNIEnv *env, jlong parserPtr, jobject origRes,
                                            jobject repRes, jobject xresources_class,
                                            jmethodID translate_attr_mid,
                                            jmethodID translate_res_mid) {
    auto parser = (android::ResXMLParser *) parserPtr;
    if (parser == nullptr) return;

    const android::ResXMLTree &mTree = parser->mTree;
    auto mResIds = (uint32_t *) mTree.mResIds;
    android::ResXMLTree_attrExt *tag;
    int attrCount;

    do {
        switch (ResXMLParser_next(parser)) {
            case android::ResXMLParser::START_TAG:
                tag = (android::ResXMLTree_attrExt *) parser->mCurExt;
                attrCount = tag->attributeCount;
                for (int idx = 0; idx < attrCount; idx++) {
                    auto attr = (android::ResXMLTree_attribute *)
                            (((const uint8_t *) tag)
                             + tag->attributeStart
                             + tag->attributeSize * idx);

                    int32_t attrNameID = ResXMLParser_getAttributeNameID(parser, idx);
                    if (attrNameID >= 0 && (size_t) attrNameID < mTree.mNumResIds &&
                        mResIds[attrNameID] >= 0x7f000000) {
                        auto attrName = mTree.mStrings.stringAt(attrNameID);
                        jint attrResID = env->CallStaticIntMethod(
                                xresources_class, translate_attr_mid,
                                env->NewString((const jchar *) attrName.data_, attrName.length_),
                                origRes);
                        if (env->ExceptionCheck()) goto leave;
                        mResIds[attrNameID] = attrResID;
                    }

                    if (attr->typedValue.dataType != android::Res_value::TYPE_REFERENCE)
                        continue;

                    jint oldValue = attr->typedValue.data;
                    if (oldValue < 0x7f000000) continue;

                    jint newValue = env->CallStaticIntMethod(
                            xresources_class, translate_res_mid, oldValue, origRes, repRes);
                    if (env->ExceptionCheck()) goto leave;
                    if (newValue != oldValue) attr->typedValue.data = newValue;
                }
                continue;
            case android::ResXMLParser::END_DOCUMENT:
            case android::ResXMLParser::BAD_DOCUMENT:
                goto leave;
            default:
                continue;
        }
    } while (true);

leave:
    ResXMLParser_restart(parser);
}
