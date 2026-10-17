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

package org.lsposed.lspd.service

import android.annotation.SuppressLint
import android.os.Parcel
import java.lang.reflect.Method

object ParcelUtils {
    private var obtainMethod: Method? = null

    @JvmStatic
    fun safeEnforceInterface(parcel: Parcel, descriptor: String): Boolean {
        return try {
            parcel.enforceInterface(descriptor)
            true
        } catch (_: Throwable) {
            false
        }
    }

    @SuppressLint("SoonBlockedPrivateApi")
    @JvmStatic
    fun fromNativePointer(ptr: Long): Parcel? {
        if (ptr == 0L) return null

        val method = obtainMethod ?: try {
            Parcel::class.java.getDeclaredMethod("obtain", java.lang.Long.TYPE).also {
                it.isAccessible = true
                obtainMethod = it
            }
        } catch (e: Throwable) {
            throw RuntimeException(e)
        }

        return try {
            method.invoke(null, ptr) as Parcel
        } catch (e: Throwable) {
            throw RuntimeException(e)
        }
    }
}
