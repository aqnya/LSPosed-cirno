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
 * Copyright (C) 2021 LSPosed Contributors
 */

package org.lsposed.lspd.util

import android.os.SystemProperties
import android.text.TextUtils
import android.util.Log
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.zone.ZoneRulesException

object Utils {
    const val LOG_TAG = "LSPosed"

    @JvmField
    val isMIUI = !TextUtils.isEmpty(SystemProperties.get("ro.miui.ui.version.name"))

    @JvmField
    val isLENOVO = !TextUtils.isEmpty(SystemProperties.get("ro.lenovo.region"))

    @JvmStatic
    fun logD(msg: Any?) {
        Log.d(LOG_TAG, msg.toString())
    }

    @JvmStatic
    fun logD(msg: String, throwable: Throwable) {
        Log.d(LOG_TAG, msg, throwable)
    }

    @JvmStatic
    fun logW(msg: String) {
        Log.w(LOG_TAG, msg)
    }

    @JvmStatic
    fun logW(msg: String, throwable: Throwable) {
        Log.w(LOG_TAG, msg, throwable)
    }

    @JvmStatic
    fun logI(msg: String) {
        Log.i(LOG_TAG, msg)
    }

    @JvmStatic
    fun logI(msg: String, throwable: Throwable) {
        Log.i(LOG_TAG, msg, throwable)
    }

    @JvmStatic
    fun logE(msg: String) {
        Log.e(LOG_TAG, msg)
    }

    @JvmStatic
    fun logE(msg: String, throwable: Throwable) {
        Log.e(LOG_TAG, msg, throwable)
    }

    @JvmStatic
    fun getZoneId(): ZoneId {
        val timezone = SystemProperties.get("persist.sys.timezone", "GMT")
        return try {
            ZoneId.of(timezone)
        } catch (_: ZoneRulesException) {
            ZoneOffset.UTC
        }
    }
}
