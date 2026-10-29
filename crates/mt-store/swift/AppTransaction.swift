import Foundation
import StoreKit

// StoreKit 2 `AppTransaction` is Swift-only, so mt-store exposes it to Rust through one C function (see build.rs).
// The App Store receipt carries no app-level original purchase date (Apple's receipt field table lists only
// the in-app purchase receipts' type 1706), which is why the paid-since check reads it from here.

private final class Result: @unchecked Sendable {
    var date = 0.0
    var sandbox: Int32 = 0
    var ok: Int32 = 0
}

@available(macOS 13.0, iOS 16.0, *)
private func load(into result: Result) async {
    do {
        guard case .verified(let transaction) = try await AppTransaction.shared else { return }
        result.date = transaction.originalPurchaseDate.timeIntervalSince1970
        result.sandbox = transaction.environment == .production ? 0 : 1
        result.ok = 1
    } catch {}
}

/// Blocks until StoreKit answers. Returns 1 and fills the out-parameters (original purchase date as Unix seconds,
/// 1 when the app runs in the sandbox / TestFlight / Xcode environment) or returns 0 when the transaction is
/// unavailable (offline on first launch, not an App Store install) or fails verification.
@_cdecl("mt_store_app_transaction")
public func mtStoreAppTransaction(_ outDate: UnsafeMutablePointer<Double>, _ outSandbox: UnsafeMutablePointer<Int32>) -> Int32 {
    guard #available(macOS 13.0, iOS 16.0, *) else { return 0 }
    let result = Result()
    let done = DispatchSemaphore(value: 0)
    Task.detached {
        await load(into: result)
        done.signal()
    }
    done.wait()
    if result.ok == 1 {
        outDate.pointee = result.date
        outSandbox.pointee = result.sandbox
    }
    return result.ok
}
