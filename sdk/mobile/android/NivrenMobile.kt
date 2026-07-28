package org.nivren

object NivrenMobile {
    private const val MAXIMUM_BYTES = 16 * 1024 * 1024

    init {
        System.loadLibrary("nivren_mobile")
    }

    external fun abiVersion(): Int
    private external fun invoke(source: ByteArray, operation: Int): ByteArray

    fun check(source: String) {
        call(source, 0)
    }

    fun format(source: String): String = call(source, 1).decodeToString()

    fun run(source: String, native: Boolean = false): String =
        call(source, if (native) 3 else 2).decodeToString()

    private fun call(source: String, operation: Int): ByteArray {
        require(abiVersion() >= 3) { "Nivren ABI 3 or newer is required" }
        val bytes = source.encodeToByteArray()
        require(bytes.size <= MAXIMUM_BYTES) { "Nivren input exceeds 16 MiB" }
        return invoke(bytes, operation).also {
            require(it.size <= MAXIMUM_BYTES) { "Nivren result exceeds 16 MiB" }
        }
    }
}
