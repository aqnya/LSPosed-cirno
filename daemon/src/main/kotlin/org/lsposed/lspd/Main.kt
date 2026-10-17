package org.lsposed.lspd

import org.lsposed.lspd.service.ServiceManager

object Main {
    @JvmStatic
    fun main(args: Array<String>) {
        ServiceManager.start(args)
    }
}
