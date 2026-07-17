// Phase-0 spike #2, SwiftUI-shell path: CryptoKit SecureEnclave.P256 signing
// the tokenfuse-cloud device-pairing protocol (07 §4.2), byte-identical to
// genaryx-signing's Rust path. `devices.rs` is the wire authority:
//   pubkey  = base64(x963Representation)            (SEC1 uncompressed)
//   sig     = base64(ECDSASignature.rawRepresentation) (raw 64-byte r||s)
//   canonical = "METHOD\nPATH\nsha256(body)hex\nTS\nNONCE"
//
// Modes:
//   (none)           local smoke: generate (enclave preferred, honest
//                    software-signed fallback), sign, verify, tamper-reject.
//   --vector         pin the two cross-language canonical vectors (must match
//                    crates/signing/src/es256.rs tests bit for bit).
//   --emit-json      print {assurance, pubkey_b64, message_b64, sig_b64} for
//                    examples/verify_es256_blob.rs to verify in Rust.
//   --cloud URL [--admin-key K]   full live pair -> signed-ack -> tamper-reject
//                    against a running tokenfuse-cloud.
//
// Fail-closed: every unexpected condition exits non-zero with a reason.

import CryptoKit
import Foundation

// MARK: - Canonical string (the Swift twin of genaryx_signing::canonical_request)

func hexLower(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

func bodySha256Hex(_ body: Data) -> String {
    hexLower(Data(SHA256.hash(data: body)))
}

func canonicalRequest(method: String, path: String, bodySha256Hex bodyHex: String, ts: String, nonce: String) -> String {
    "\(method)\n\(path)\n\(bodyHex)\n\(ts)\n\(nonce)"
}

// MARK: - Signer (Secure Enclave preferred, honest software-signed fallback)

enum Signer {
    case enclave(SecureEnclave.P256.Signing.PrivateKey)
    case software(P256.Signing.PrivateKey)

    /// The journal/UI label (06 §3): exactly two honest values.
    var assuranceLabel: String {
        switch self {
        case .enclave: return "secure-enclave"
        case .software: return "software-signed"
        }
    }

    var publicKeyX963: Data {
        switch self {
        case .enclave(let key): return key.publicKey.x963Representation
        case .software(let key): return key.publicKey.x963Representation
        }
    }

    /// Raw 64-byte r||s (IEEE P1363) - CryptoKit's rawRepresentation IS the
    /// wire form; unlike SecKey there is no DER conversion on this path.
    func signRaw(_ message: Data) throws -> Data {
        switch self {
        case .enclave(let key): return try key.signature(for: message).rawRepresentation
        case .software(let key): return try key.signature(for: message).rawRepresentation
        }
    }

    func verify(_ signatureRaw: Data, for message: Data) -> Bool {
        guard let sig = try? P256.Signing.ECDSASignature(rawRepresentation: signatureRaw) else {
            return false
        }
        switch self {
        case .enclave(let key): return key.publicKey.isValidSignature(sig, for: message)
        case .software(let key): return key.publicKey.isValidSignature(sig, for: message)
        }
    }

    /// Prefer the enclave; fall back to a software key with the honest label
    /// and the refusal reason surfaced, never silently.
    static func generatePreferringEnclave() -> (Signer, String?) {
        if SecureEnclave.isAvailable {
            do {
                return (.enclave(try SecureEnclave.P256.Signing.PrivateKey()), nil)
            } catch {
                return (.software(P256.Signing.PrivateKey()), "enclave keygen failed: \(error)")
            }
        }
        return (.software(P256.Signing.PrivateKey()), "SecureEnclave.isAvailable == false")
    }
}

// MARK: - Pinned cross-language vectors (MUST match crates/signing/src/es256.rs)

let v1Method = "POST"
let v1Path = "/v1/runs/spike2-e2e/budget"
let v1Body = Data("{\"budget_usd\":12.5,\"note\":\"обмеження діє\"}".utf8)
let v1Ts = "1758000000"
let v1Nonce = "genaryx-spike2-nonce"
let v1BodySha256 = "94443f9c3dbe6095049a04c7c23436f246d12566f1108d6c1c5df1bf373405b9"
let v1CanonicalSha256 = "66c4919da908f16b8ea5a7cdc2a51c7a271653d4a6a0cb9f634ff64de9ef9f2a"
let v2Path = "/v1/runs/spike2-e2e/kill"
let v2CanonicalSha256 = "4bbe4ceedc64d8bf1191a48cd8a98b9b8482ce5ecb948a1df65d6dd29ed27aa8"

func v1Canonical() -> String {
    canonicalRequest(method: v1Method, path: v1Path, bodySha256Hex: bodySha256Hex(v1Body), ts: v1Ts, nonce: v1Nonce)
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FAIL: \(message)\n".utf8))
    exit(1)
}

func runVector() {
    guard v1Body.count == 54 else { fail("vector body must be 54 UTF-8 bytes, got \(v1Body.count)") }
    let bodyHex = bodySha256Hex(v1Body)
    guard bodyHex == v1BodySha256 else { fail("body sha256 drifted: \(bodyHex)") }
    let c1 = v1Canonical()
    let c1Sha = hexLower(Data(SHA256.hash(data: Data(c1.utf8))))
    guard c1Sha == v1CanonicalSha256 else { fail("vector 1 canonical drifted: \(c1Sha)") }
    let c2 = canonicalRequest(method: v1Method, path: v2Path, bodySha256Hex: bodySha256Hex(Data()), ts: v1Ts, nonce: v1Nonce)
    let c2Sha = hexLower(Data(SHA256.hash(data: Data(c2.utf8))))
    guard c2Sha == v2CanonicalSha256 else { fail("vector 2 canonical drifted: \(c2Sha)") }
    print("vector 1 canonical (verbatim):\n---\n\(c1)\n---")
    print("vector 1 sha256: \(c1Sha)")
    print("vector 2 sha256: \(c2Sha)")
    print("PASS: both cross-language canonical vectors byte-identical to the Rust pins")
}

func runLocalSmoke() {
    print("SecureEnclave.isAvailable = \(SecureEnclave.isAvailable)")
    let (signer, fallbackReason) = Signer.generatePreferringEnclave()
    if let reason = fallbackReason {
        print("signer: \(signer.assuranceLabel) (fallback: \(reason))")
    } else {
        print("signer: \(signer.assuranceLabel) (P-256 in the Apple Secure Enclave)")
    }
    let pub = signer.publicKeyX963
    guard pub.count == 65, pub.first == 0x04 else { fail("public key is not an X9.63 uncompressed point") }
    print("pubkey x963 b64: \(pub.base64EncodedString())")

    let message = Data(v1Canonical().utf8)
    guard let sig = try? signer.signRaw(message) else { fail("signing failed") }
    guard sig.count == 64 else { fail("signature is not raw 64-byte r||s: \(sig.count) bytes") }
    print("signature raw r||s b64: \(sig.base64EncodedString())")
    guard signer.verify(sig, for: message) else { fail("genuine signature did not verify") }
    print("local verify: OK")
    var tampered = message
    tampered[0] ^= 0x01
    guard !signer.verify(sig, for: tampered) else { fail("tampered message verified - broken") }
    print("tamper-reject: OK")
    print("PASS: \(signer.assuranceLabel) generate + sign + verify + tamper-reject")
}

func runEmitJson() {
    let (signer, _) = Signer.generatePreferringEnclave()
    let message = Data(v1Canonical().utf8)
    guard let sig = try? signer.signRaw(message) else { fail("signing failed") }
    let blob: [String: String] = [
        "assurance": signer.assuranceLabel,
        "pubkey_b64": signer.publicKeyX963.base64EncodedString(),
        "message_b64": message.base64EncodedString(),
        "sig_b64": sig.base64EncodedString(),
    ]
    let data = try! JSONSerialization.data(withJSONObject: blob, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
}

// MARK: - Live cloud driver

struct Http {
    let base: URL

    func request(_ method: String, _ path: String, bearer: String, body: Data,
                 signedHeaders: [String: String] = [:]) -> (Int, String) {
        var req = URLRequest(url: base.appendingPathComponent(path))
        req.httpMethod = method
        req.httpBody = body
        req.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        for (k, v) in signedHeaders { req.setValue(v, forHTTPHeaderField: k) }
        let sem = DispatchSemaphore(value: 0)
        var status = -1
        var text = ""
        URLSession.shared.dataTask(with: req) { data, resp, err in
            if let err = err { text = "transport error: \(err)" }
            if let http = resp as? HTTPURLResponse { status = http.statusCode }
            if let data = data { text = String(data: data, encoding: .utf8) ?? "<binary>" }
            sem.signal()
        }.resume()
        sem.wait()
        return (status, text)
    }
}

var liveFailures = 0
func checkLive(_ what: String, got: Int, want: Int, body: String) {
    let ok = got == want
    if !ok { liveFailures += 1 }
    print("  [\(ok ? "PASS" : "FAIL")] \(what): HTTP \(got) (want \(want)) \(body)")
}

func signedHeaders(signer: Signer, deviceId: String, method: String, path: String,
                   body: Data, ts: String, nonce: String) -> [String: String] {
    let canonical = canonicalRequest(method: method, path: path, bodySha256Hex: bodySha256Hex(body), ts: ts, nonce: nonce)
    guard let sig = try? signer.signRaw(Data(canonical.utf8)) else { fail("signing failed") }
    return [
        "X-Fuse-Device": deviceId,
        "X-Fuse-TS": ts,
        "X-Fuse-Nonce": nonce,
        "X-Fuse-Sig": sig.base64EncodedString(),
    ]
}

func runCloud(baseUrl: String, adminKey: String) {
    guard let base = URL(string: baseUrl) else { fail("bad --cloud URL: \(baseUrl)") }
    let http = Http(base: base)
    print("== spike #2 signed-ack (Swift/CryptoKit) vs live tokenfuse-cloud at \(baseUrl) ==")

    let (signer, fallbackReason) = Signer.generatePreferringEnclave()
    print("signer: \(signer.assuranceLabel)\(fallbackReason.map { " (fallback: \($0))" } ?? "")")

    let (hs, hb) = http.request("GET", "healthz", bearer: adminKey, body: Data())
    checkLive("healthz", got: hs, want: 200, body: hb)

    // 1) Admin issues a pairing code.
    let (ns, nb) = http.request("POST", "v1/pair/new", bearer: adminKey, body: Data("{}".utf8))
    checkLive("POST /v1/pair/new (admin org key)", got: ns, want: 200, body: nb)
    guard let newJson = try? JSONSerialization.jsonObject(with: Data(nb.utf8)) as? [String: Any],
          let code = newJson["code"] as? String else { fail("no pairing code in: \(nb)") }

    // 2) Redeem it with the device public key.
    let pairBody: [String: String] = [
        "code": code,
        "pubkey_b64": signer.publicKeyX963.base64EncodedString(),
        "platform": "macos",
        "name": "genaryx-spike2-swift (\(signer.assuranceLabel))",
    ]
    let (ps, pb) = http.request("POST", "v1/pair", bearer: adminKey,
                                body: try! JSONSerialization.data(withJSONObject: pairBody))
    checkLive("POST /v1/pair (redeem code + pubkey)", got: ps, want: 200, body: pb)
    guard let pairJson = try? JSONSerialization.jsonObject(with: Data(pb.utf8)) as? [String: Any],
          let deviceId = pairJson["device_id"] as? String,
          let deviceToken = pairJson["device_token"] as? String else { fail("no device in: \(pb)") }
    print("paired: device_id=\(deviceId)")

    // 3) Genuine signed mutation: THE signed-ack.
    let run = "spike2-swift-\(UInt32.random(in: 0x1000_0000...0xffff_ffff))"
    let killPath = "/v1/runs/\(run)/kill"
    let now = String(Int(Date().timeIntervalSince1970))
    let h1 = signedHeaders(signer: signer, deviceId: deviceId, method: "POST", path: killPath,
                           body: Data(), ts: now, nonce: "swift-n1-\(UUID().uuidString)")
    let (ks, kb) = http.request("POST", killPath, bearer: deviceToken, body: Data(), signedHeaders: h1)
    checkLive("signed kill (genuine)", got: ks, want: 200, body: kb)

    // 4) Genuine signed mutation with the pinned multibyte UTF-8 body.
    let budgetPath = "/v1/runs/\(run)/budget"
    let h2 = signedHeaders(signer: signer, deviceId: deviceId, method: "POST", path: budgetPath,
                           body: v1Body, ts: now, nonce: "swift-n2-\(UUID().uuidString)")
    let (bs, bb) = http.request("POST", budgetPath, bearer: deviceToken, body: v1Body, signedHeaders: h2)
    checkLive("signed budget (genuine, UTF-8 body)", got: bs, want: 200, body: bb)

    // 5) Tampered signature must be rejected.
    var badHeaders = signedHeaders(signer: signer, deviceId: deviceId, method: "POST", path: killPath,
                                   body: Data(), ts: now, nonce: "swift-n3-\(UUID().uuidString)")
    var sigBytes = Data(base64Encoded: badHeaders["X-Fuse-Sig"]!)!
    sigBytes[7] ^= 0x01
    badHeaders["X-Fuse-Sig"] = sigBytes.base64EncodedString()
    let (ts1, tb1) = http.request("POST", killPath, bearer: deviceToken, body: Data(), signedHeaders: badHeaders)
    checkLive("corrupted signature rejected", got: ts1, want: 403, body: tb1)

    // 6) Replayed nonce must be rejected (fresh valid signature, spent nonce).
    let replayNonce = "swift-replay-\(UUID().uuidString)"
    let r1 = signedHeaders(signer: signer, deviceId: deviceId, method: "POST", path: killPath,
                           body: Data(), ts: now, nonce: replayNonce)
    let (rs1, rb1) = http.request("POST", killPath, bearer: deviceToken, body: Data(), signedHeaders: r1)
    checkLive("first use of nonce accepted", got: rs1, want: 200, body: rb1)
    let r2 = signedHeaders(signer: signer, deviceId: deviceId, method: "POST", path: killPath,
                           body: Data(), ts: now, nonce: replayNonce)
    let (rs2, rb2) = http.request("POST", killPath, bearer: deviceToken, body: Data(), signedHeaders: r2)
    checkLive("replayed nonce rejected", got: rs2, want: 403, body: rb2)

    if liveFailures == 0 {
        print("== SIGNED-ACK PROVEN (Swift/CryptoKit \(signer.assuranceLabel)) ==")
    } else {
        print("== \(liveFailures) check(s) FAILED ==")
        exit(1)
    }
}

// MARK: - Entry

let args = CommandLine.arguments.dropFirst()
switch args.first {
case "--vector":
    runVector()
case "--emit-json":
    runEmitJson()
case "--cloud":
    guard args.count >= 2 else { fail("--cloud needs a base URL") }
    let url = Array(args)[1]
    var adminKey = "devkey"
    if let i = args.firstIndex(of: "--admin-key"), args.indices.contains(i + 1) {
        adminKey = args[i + 1]
    }
    runCloud(baseUrl: url, adminKey: adminKey)
case nil:
    runLocalSmoke()
case .some(let other):
    fail("unknown mode: \(other)")
}
