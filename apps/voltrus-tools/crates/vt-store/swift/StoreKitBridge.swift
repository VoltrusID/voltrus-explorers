// StoreKit 2 for the Mac App Store build of Voltrus Tools, behind a C ABI.
//
// Built by ../build.rs into a static library (store builds on macOS only). Every call blocks its
// caller until StoreKit answers, so Rust calls them from a background thread, never the UI thread.
// Answers are JSON strings allocated with strdup; Rust hands each back to vt_sk_free.
//
// Only transactions StoreKit verified (signed by the App Store for this app and device) count:
// unverified or revoked ones are skipped, so a refund removes the entitlement.

import AppKit
import Foundation
import StoreKit

private final class Box: @unchecked Sendable {
    var value: [String: Any] = [:]
}

private func encode(_ object: [String: Any]) -> UnsafeMutablePointer<CChar> {
    let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
    // strdup fails only when out of memory.
    return strdup(String(decoding: data, as: UTF8.self))!
}

/// Runs `body` on the Swift concurrency pool and waits for it on the calling (Rust) thread.
private func blocking(_ body: @escaping @Sendable () async -> [String: Any]) -> UnsafeMutablePointer<CChar> {
    let box = Box()
    let done = DispatchSemaphore(value: 0)
    Task.detached {
        box.value = await body()
        done.signal()
    }
    done.wait()
    return encode(box.value)
}

private func seconds(_ date: Date) -> Int64 {
    Int64(date.timeIntervalSince1970.rounded(.down))
}

private func productIDs(_ json: UnsafePointer<CChar>) -> [String] {
    let data = Data(String(cString: json).utf8)
    return (try? JSONSerialization.jsonObject(with: data)) as? [String] ?? []
}

/// The verified, unrevoked non-consumables this Apple ID owns. StoreKit answers from its on-device
/// cache, so this also works offline.
private func entitlements() async -> [[String: Any]] {
    var owned: [[String: Any]] = []
    for await result in Transaction.currentEntitlements {
        guard case .verified(let transaction) = result, transaction.revocationDate == nil else { continue }
        owned.append([
            "product": transaction.productID,
            // The first purchase of this product on this Apple ID: a re-download or restore keeps it.
            "original": seconds(transaction.originalPurchaseDate),
            // When the App Store signed this record: a time the local clock cannot move back past.
            "signed": seconds(transaction.signedDate),
        ])
    }
    return owned
}

private func finishUnfinished() async {
    for await result in Transaction.unfinished {
        if case .verified(let transaction) = result {
            await transaction.finish()
        }
    }
}

private func snapshot(_ ids: [String]) async -> [String: Any] {
    await finishUnfinished()
    var answer: [String: Any] = ["entitlements": await entitlements()]
    do {
        var prices: [String: String] = [:]
        for product in try await Product.products(for: ids) {
            prices[product.id] = product.displayPrice
        }
        answer["prices"] = prices
    } catch {
        // Offline: entitlements still count, prices are simply unknown.
        answer["prices"] = [String: String]()
    }
    return answer
}

/// `{"entitlements":[{"product","original","signed"}],"prices":{id: "$49.99"}}` for the product ids
/// in `ids_json` (a JSON array of strings).
@_cdecl("vt_sk_snapshot")
public func vt_sk_snapshot(_ ids_json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let ids = productIDs(ids_json)
    return blocking { await snapshot(ids) }
}

/// Buys one product. `{"result":"purchased","original":<unix>}`, `{"result":"pending"}` (Ask to Buy),
/// `{"result":"cancelled"}` or `{"result":"error","message":...}`.
@_cdecl("vt_sk_purchase")
public func vt_sk_purchase(_ id: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let productID = String(cString: id)
    return blocking {
        do {
            guard let product = try await Product.products(for: [productID]).first else {
                return ["result": "error", "message": "The App Store doesn't offer this item right now. Try again later."]
            }
            switch try await product.purchase() {
            case .success(.verified(let transaction)):
                await transaction.finish()
                return ["result": "purchased", "original": seconds(transaction.originalPurchaseDate)]
            case .success(.unverified(_, let error)):
                return ["result": "error", "message": "The App Store could not verify the purchase: \(error.localizedDescription)"]
            case .pending:
                return ["result": "pending"]
            case .userCancelled:
                return ["result": "cancelled"]
            @unknown default:
                return ["result": "cancelled"]
            }
        } catch {
            return ["result": "error", "message": error.localizedDescription]
        }
    }
}

/// Restore purchases: syncs this Apple ID's transactions with the App Store (it may ask the user to
/// sign in), then answers like vt_sk_snapshot, or `{"error": message}`.
@_cdecl("vt_sk_restore")
public func vt_sk_restore(_ ids_json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let ids = productIDs(ids_json)
    return blocking {
        do {
            try await AppStore.sync()
        } catch StoreKitError.userCancelled {
            return ["error": "cancelled"]
        } catch {
            return ["error": error.localizedDescription]
        }
        return await snapshot(ids)
    }
}

/// Calls `callback` (on a StoreKit thread) whenever a transaction arrives outside a purchase call:
/// an Ask to Buy approval, a purchase on another Mac, a refund. Call once.
@_cdecl("vt_sk_listen")
public func vt_sk_listen(_ callback: @escaping @convention(c) () -> Void) {
    Task.detached {
        for await result in Transaction.updates {
            if case .verified(let transaction) = result {
                await transaction.finish()
            }
            callback()
        }
    }
}

/// Asks StoreKit to show the system rating prompt. Apple decides whether it appears (at most three
/// times a year); nothing is returned. Returns at once: the request runs on the main actor. Uses
/// `AppStore.requestReview(in:)` with the key window's view controller when there is one, and
/// `SKStoreReviewController.requestReview()` for a window without one.
@_cdecl("vt_sk_request_review")
public func vt_sk_request_review() {
    Task { @MainActor in
        if #available(macOS 13.0, *), let controller = NSApplication.shared.keyWindow?.contentViewController {
            AppStore.requestReview(in: controller)
        } else {
            SKStoreReviewController.requestReview()
        }
    }
}

@_cdecl("vt_sk_free")
public func vt_sk_free(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}
