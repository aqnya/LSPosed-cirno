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
 * Copyright (C) 2021 - 2022 LSPosed Contributors
 */

package org.lsposed.lspd.impl;

import android.os.Bundle;
import android.os.RemoteException;

import org.lsposed.lspd.models.Module;
import org.lsposed.lspd.service.ILSPHotReloadCallback;

/**
 * Process-side endpoint of the daemon's hot reload requests. Registered through
 * {@link org.lsposed.lspd.core.ApplicationServiceClient} at startup.
 */
public class LSPosedHotReloadCallback extends ILSPHotReloadCallback.Stub {
    @Override
    public Bundle hotReload(Module module, Bundle extras) throws RemoteException {
        return LSPosedContext.handleHotReload(module, extras);
    }
}
