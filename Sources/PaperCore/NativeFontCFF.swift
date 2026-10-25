import Foundation

/// A bounded CFF1 directory/charset reader, written from the published CFF
/// binary format. It never interprets, converts, or replaces glyph outlines.
enum NativeFontCFF {
    enum Invalid: Error { case structure }
    struct Charset {
        let registry: String
        let ordering: String
        let supplement: Int
        let cids: [Int]
        let denseProgram: Data
    }

    static func charset(_ data: Data) throws -> Charset {
        var reader = Reader(bytes: Array(data))
        guard reader.bytes.count >= 4, reader.bytes.count <= 16 * 1024 * 1024,
              reader.bytes[0] == 1, reader.bytes[1] == 0,
              reader.bytes[2] >= 4, (1...4).contains(reader.bytes[3]) else { throw Invalid.structure }
        reader.at = Int(reader.bytes[2])
        let names = try reader.index(), tops = try reader.index(), strings = try reader.index()
        _ = try reader.index() // Global subroutines; directory bounds only.
        var protectedRanges = [0..<reader.at]
        guard names.count == 1, tops.count == 1 else { throw Invalid.structure }
        let top = try dictionary(tops[0])
        guard let ros = top[1230], ros.count == 3,
              ros.allSatisfy({ $0.isFinite && $0 >= 0 && $0 <= 65535 && $0.rounded() == $0 }),
              let registry = string(Int(ros[0]), strings), let ordering = string(Int(ros[1]), strings),
              ros[2] >= 0, ros[2] <= 65535,
              let chars = integer(top[17]), let charset = integer(top[15]), charset > 2,
              let fdArray = integer(top[1236]), let fdSelect = integer(top[1237]),
              top[18] == nil,
              top[1207] == nil || top[1207] == [0.001, 0, 0, 0.001, 0, 0] else { throw Invalid.structure }
        reader.at = chars
        let glyphs = try reader.index()
        protectedRanges.append(chars..<reader.at)
        guard glyphs.count > 1, glyphs.count <= 65535 else { throw Invalid.structure }
        reader.at = fdArray
        let dictionaries = try reader.index()
        protectedRanges.append(fdArray..<reader.at)
        guard !dictionaries.isEmpty, dictionaries.count <= 256 else { throw Invalid.structure }
        for bytes in dictionaries {
            let font = try dictionary(bytes)
            if let entry = font[18] {
                guard entry.count == 2, let size = integer([entry[0]]), let offset = integer([entry[1]]),
                      offset <= data.count, size <= data.count - offset else { throw Invalid.structure }
                let privateData = Array(data[offset..<(offset + size)])
                protectedRanges.append(offset..<(offset + size))
                let values = try dictionary(privateData)
                if let local = values[19] {
                    guard let relative = integer(local), relative <= data.count - offset else { throw Invalid.structure }
                    let position = offset + relative
                    reader.at = position; _ = try reader.index()
                    protectedRanges.append(position..<reader.at)
                }
            }
        }
        reader.at = fdSelect
        switch try reader.byte() {
        case 0:
            for _ in glyphs.indices { guard Int(try reader.byte()) < dictionaries.count else { throw Invalid.structure } }
        case 3:
            let count = try reader.uint(2)
            guard count > 0, count <= glyphs.count else { throw Invalid.structure }
            var previous = -1
            for index in 0..<count {
                let first = try reader.uint(2), fd = Int(try reader.byte())
                guard first > previous, first < glyphs.count, fd < dictionaries.count,
                      index > 0 || first == 0 else { throw Invalid.structure }
                previous = first
            }
            guard try reader.uint(2) == glyphs.count else { throw Invalid.structure }
        default: throw Invalid.structure
        }
        protectedRanges.append(fdSelect..<reader.at)
        reader.at = charset
        let format = try reader.byte()
        var cids = [0], dense = data
        func writeCID(_ cid: Int, at: Int) {
            dense[at] = UInt8(cid >> 8); dense[at + 1] = UInt8(cid & 255)
        }
        switch format {
        case 0:
            while cids.count < glyphs.count {
                let position = reader.at, cid = try reader.uint(2)
                writeCID(cids.count, at: position)
                cids.append(cid)
            }
        case 1, 2:
            while cids.count < glyphs.count {
                let position = reader.at
                let first = try reader.uint(2), remaining = try reader.uint(format == 1 ? 1 : 2)
                guard first > 0, first <= 65535 - remaining,
                      remaining < glyphs.count - cids.count else { throw Invalid.structure }
                writeCID(cids.count, at: position)
                cids += Array(first...(first + remaining))
            }
        default: throw Invalid.structure
        }
        let charsetRange = charset..<reader.at
        guard Set(cids).count == cids.count,
              protectedRanges.allSatisfy({ !$0.overlaps(charsetRange) }) else { throw Invalid.structure }
        return Charset(registry: registry, ordering: ordering, supplement: Int(ros[2]), cids: cids, denseProgram: dense)
    }

    private static func integer(_ values: [Double]?) -> Int? {
        guard let values, values.count == 1, let value = values.first, value.isFinite,
              value >= 0, value <= Double(16 * 1024 * 1024), value.rounded() == value else { return nil }
        return Int(value)
    }

    private static func string(_ sid: Int, _ strings: [[UInt8]]) -> String? {
        // CID ROS custom strings are required here. Predefined strings outside
        // this supported set stay unknown; no guessed encoding is introduced.
        guard sid >= 391, sid - 391 < strings.count else { return nil }
        let bytes = strings[sid - 391]
        guard !bytes.isEmpty, bytes.count <= 127,
              bytes.allSatisfy({ $0 >= 33 && $0 <= 126 && ![40, 41, 92].contains($0) }) else { return nil }
        return String(bytes: bytes, encoding: .ascii)
    }

    private static func dictionary(_ bytes: [UInt8]) throws -> [Int: [Double]] {
        var reader = Reader(bytes: bytes), stack: [Double] = [], output: [Int: [Double]] = [:]
        while reader.at < bytes.count {
            let first = try reader.byte()
            let value: Double
            switch first {
            case 32...246: value = Double(Int(first) - 139)
            case 247...250: value = Double((Int(first) - 247) * 256 + Int(try reader.byte()) + 108)
            case 251...254: value = Double(-(Int(first) - 251) * 256 - Int(try reader.byte()) - 108)
            case 28: value = Double(Int16(bitPattern: UInt16(try reader.uint(2))))
            case 29: value = Double(Int32(bitPattern: UInt32(try reader.uint(4))))
            case 30:
                var text = "", ended = false
                while !ended {
                    guard text.count <= 64 else { throw Invalid.structure }
                    let pair = try reader.byte()
                    for nibble in [pair >> 4, pair & 15] {
                        switch nibble {
                        case 0...9: text += String(nibble)
                        case 10: text += "."
                        case 11: text += "E"
                        case 12: text += "E-"
                        case 14: text += "-"
                        case 15: ended = true
                        default: throw Invalid.structure
                        }
                        if ended { break }
                    }
                }
                guard let real = Double(text), real.isFinite else { throw Invalid.structure }
                value = real
            case 0...21:
                let key = first == 12 ? 1200 + Int(try reader.byte()) : Int(first)
                guard output[key] == nil else { throw Invalid.structure }
                output[key] = stack; stack = []; continue
            default: throw Invalid.structure
            }
            stack.append(value)
            guard stack.count <= 48 else { throw Invalid.structure }
        }
        guard stack.isEmpty else { throw Invalid.structure }
        return output
    }

    private struct Reader {
        let bytes: [UInt8]
        var at = 0
        mutating func byte() throws -> UInt8 {
            guard at >= 0, at < bytes.count else { throw Invalid.structure }
            defer { at += 1 }; return bytes[at]
        }
        mutating func uint(_ size: Int) throws -> Int {
            guard (1...4).contains(size) else { throw Invalid.structure }
            var result = 0
            for _ in 0..<size { result = result * 256 + Int(try byte()) }
            return result
        }
        mutating func index() throws -> [[UInt8]] {
            let count = try uint(2)
            guard count > 0 else { return [] }
            let size = Int(try byte())
            guard (1...4).contains(size), count <= 65535 else { throw Invalid.structure }
            var offsets: [Int] = []
            for _ in 0...count { offsets.append(try uint(size)) }
            guard offsets[0] == 1 else { throw Invalid.structure }
            let start = at
            guard let last = offsets.last, last >= 1, last - 1 <= bytes.count - start else { throw Invalid.structure }
            var values: [[UInt8]] = []
            for index in 0..<count {
                guard offsets[index] <= offsets[index+1] else { throw Invalid.structure }
                values.append(Array(bytes[(start + offsets[index] - 1)..<(start + offsets[index+1] - 1)]))
            }
            at = start + last - 1
            return values
        }
    }
}
