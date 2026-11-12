import Foundation

/// A bounded cache of values worked out from a string. The room view rebuilds every message row each time it redraws,
/// so work that depends only on a message's text belongs here rather than in a view's initialiser or body.
final class ParseCache<Value>: @unchecked Sendable {
    private final class Box {
        let value: Value
        init(_ value: Value) { self.value = value }
    }

    private let storage = NSCache<NSString, Box>()
    private let lock = NSLock()
    private var missCount = 0

    /// How many lookups had to build their value. Tests read it to prove a repeat lookup does no work.
    var misses: Int {
        lock.lock(); defer { lock.unlock() }
        return missCount
    }

    /// Each entry costs its key's UTF-8 length, so a few large messages, such as charts, cannot hold the whole budget.
    init(countLimit: Int, byteLimit: Int) {
        storage.countLimit = countLimit
        storage.totalCostLimit = byteLimit
    }

    func contains(_ key: String) -> Bool {
        storage.object(forKey: key as NSString) != nil
    }

    func removeAll() {
        storage.removeAllObjects()
    }

    func value(for key: String, build: (String) -> Value) -> Value {
        if let hit = storage.object(forKey: key as NSString) { return hit.value }
        let value = build(key)
        storage.setObject(Box(value), forKey: key as NSString, cost: key.utf8.count)
        lock.lock(); missCount += 1; lock.unlock()
        return value
    }
}

/// The caches that hold message text as the feed draws it. An unlocked room's text is plaintext, so whatever drops a
/// store's decrypted text, or the store itself, must clear these too.
enum RenderCaches {
    static func clearPlaintext() {
        MessageTextLayout.cache.removeAll()
        MessageContentParser.inlineCache.removeAll()
        ChartMessage.decodedImages.removeAllObjects()
    }
}
