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

package org.lsposed.manager.util

import android.util.Log
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.Request
import okio.buffer
import okio.sink
import org.lsposed.manager.App
import org.lsposed.manager.ConfigManager
import java.io.File
import java.io.IOException
import java.time.Instant
import java.util.Locale

object UpdateUtil {
    // One process-wide scope is enough: these are one-shot background checks.
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @JvmStatic
    fun loadRemoteVersion() {
        scope.launch {
            val request = Request.Builder()
                .url("https://api.github.com/repos/aqnya/LSPosed-cirno/releases/latest")
                .addHeader("Accept", "application/vnd.github.v3+json")
                .build()

            val response = try {
                App.getOkHttpClient().newCall(request).execute()
            } catch (e: IOException) {
                onFailure(e)
                return@launch
            }

            response.use {
                if (!it.isSuccessful) return@launch
                val body = it.body ?: return@launch
                try {
                    val api = if (ConfigManager.isBinderAlive()) ConfigManager.getApi() else "zygisk"
                    val info = JsonParser.parseReader(body.charStream()).asJsonObject
                    val notes = info.get("body").asString
                    val assets = info.getAsJsonArray("assets")
                    for (i in 0 until assets.size()) {
                        checkAssets(assets[i].asJsonObject, notes, api.lowercase(Locale.ROOT))
                    }
                } catch (t: Throwable) {
                    Log.e(App.TAG, t.message, t)
                }
            }
        }
    }

    private fun onFailure(e: IOException) {
        Log.e(App.TAG, "loadRemoteVersion: ${e.message}")
        val pref = App.getPreferences()
        if (pref.getBoolean("checked", false)) return
        pref.edit().putBoolean("checked", true).apply()
    }

    private suspend fun checkAssets(assets: JsonObject, releaseNotes: String, api: String) {
        val pref = App.getPreferences()
        val name = assets.get("name").asString
        val splitName = name.split("-")
        if (splitName[3] != api) return
        pref.edit()
            .putInt("latest_version", splitName[2].toInt())
            .putLong("latest_check", Instant.now().epochSecond)
            .putString("release_notes", releaseNotes)
            .putString("zip_file", null)
            .putBoolean("checked", true)
            .apply()
        val updatedAt = Instant.parse(assets.get("updated_at").asString)
        val downloadUrl = assets.get("browser_download_url").asString
        val zipTime = pref.getLong("zip_time", 0)
        if (updatedAt != Instant.ofEpochSecond(zipTime)) {
            val zip = downloadNewZip(downloadUrl, name)
            val size = assets.get("size").asLong
            if (zip != null && zip.length() == size) {
                pref.edit()
                    .putLong("zip_time", updatedAt.epochSecond)
                    .putString("zip_file", zip.absolutePath)
                    .apply()
            }
        }
    }

    private suspend fun downloadNewZip(url: String, name: String): File? = withContext(Dispatchers.IO) {
        val request = Request.Builder().url(url).build()
        val zip = File(App.getInstance().cacheDir, name)
        try {
            App.getOkHttpClient().newCall(request).execute().use { response ->
                val body = response.body
                if (!response.isSuccessful || body == null) {
                    return@withContext null
                }
                body.source().use { source ->
                    zip.sink().buffer().use { sink ->
                        sink.writeAll(source)
                    }
                }
            }
        } catch (e: IOException) {
            Log.e(App.TAG, "downloadNewZipSync: ${e.message}")
            return@withContext null
        }
        zip
    }

    @JvmStatic
    fun needUpdate(): Boolean = false

    @JvmStatic
    fun canInstall(): Boolean {
        if (!ConfigManager.isBinderAlive()) return false
        val pref = App.getPreferences()
        val zip = pref.getString("zip_file", null)
        return zip != null && File(zip).isFile
    }
}
