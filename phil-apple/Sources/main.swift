import Foundation
import FoundationModels

/// phil-apple: thin helper that exposes Apple Intelligence to phil (Rust).
///
/// Protocol (one JSON object per line on stdin, one per line on stdout):
///   Request:  {"system":"...","prompt":"...","temperature":0.1,"max_tokens":2048}
///   Response: {"text":"..."}
///   Error:    {"error":"..."}
///
/// Exits after processing all stdin lines (EOF closes).

struct Request: Decodable {
    let system: String?
    let prompt: String
    let temperature: Double?
    let maxTokens: Int?

    enum CodingKeys: String, CodingKey {
        case system, prompt, temperature
        case maxTokens = "max_tokens"
    }
}

struct SuccessResponse: Encodable {
    let text: String
}

struct ErrorResponse: Encodable {
    let error: String
}

@main
struct PhilApple {
    static func main() async {
        // Handle --ping for capability detection
        if CommandLine.arguments.contains("--ping") {
            print("{\"status\":\"ok\",\"model\":\"apple-foundationmodel\",\"context_window\":4096}")
            return
        }

        let model = SystemLanguageModel.default

        // Read JSON lines from stdin
        while let line = readLine(strippingNewline: true) {
            guard !line.isEmpty else { continue }

            do {
                guard let data = line.data(using: .utf8) else {
                    let resp = ErrorResponse(error: "invalid utf8")
                    printJSON(resp)
                    continue
                }

                let req = try JSONDecoder().decode(Request.self, from: data)

                var instructions = ""
                if let sys = req.system, !sys.isEmpty {
                    instructions = sys
                }

                let session = LanguageModelSession(model: model, instructions: instructions)
                let response = try await session.respond(to: req.prompt)
                let text = response.content

                printJSON(SuccessResponse(text: text))
            } catch {
                printJSON(ErrorResponse(error: "\(error)"))
            }
        }
    }

    static func printJSON<T: Encodable>(_ value: T) {
        do {
            let data = try JSONEncoder().encode(value)
            if let str = String(data: data, encoding: .utf8) {
                print(str)
                fflush(stdout)
            }
        } catch {
            print("{\"error\":\"json encode failed\"}")
            fflush(stdout)
        }
    }
}
