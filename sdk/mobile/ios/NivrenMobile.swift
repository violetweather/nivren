import CNivren
import Foundation

public struct NivrenFailure: Error, Sendable {
    public let status: UInt32
    public let message: String
}

public enum NivrenMobile {
    public static let maximumBytes = 16 * 1024 * 1024

    public static func check(_ source: String) throws {
        _ = try invoke(source, operation: nivren_check_utf8)
    }

    public static func format(_ source: String) throws -> String {
        try decode(try invoke(source, operation: nivren_format_utf8))
    }

    public static func run(_ source: String, native: Bool = false) throws -> String {
        let operation = native ? nivren_run_native_utf8 : nivren_run_utf8
        return try decode(try invoke(source, operation: operation))
    }

    private static func invoke(
        _ source: String,
        operation: (UnsafePointer<UInt8>?, Int) -> NivrenBuffer
    ) throws -> Data {
        guard nivren_abi_version() >= 3 else {
            throw NivrenFailure(status: 2, message: "Nivren ABI 3 or newer is required")
        }
        let bytes = Array(source.utf8)
        guard bytes.count <= maximumBytes else {
            throw NivrenFailure(status: 2, message: "Nivren input exceeds 16 MiB")
        }
        var buffer = bytes.withUnsafeBufferPointer { pointer in
            operation(pointer.baseAddress, pointer.count)
        }
        defer { nivren_buffer_free(buffer) }
        guard buffer.length <= maximumBytes, buffer.data != nil || buffer.length == 0 else {
            throw NivrenFailure(status: 3, message: "Nivren returned an invalid buffer")
        }
        let data = buffer.length == 0
            ? Data()
            : Data(bytes: buffer.data!, count: buffer.length)
        guard buffer.status == 0 else {
            throw NivrenFailure(
                status: buffer.status,
                message: String(data: data, encoding: .utf8) ?? "Nivren returned non-UTF-8 failure data"
            )
        }
        return data
    }

    private static func decode(_ data: Data) throws -> String {
        guard let value = String(data: data, encoding: .utf8) else {
            throw NivrenFailure(status: 3, message: "Nivren returned non-UTF-8 data")
        }
        return value
    }
}
