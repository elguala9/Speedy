import Foundation

// MARK: - Types

enum HTTPMethod: String {
    case get    = "GET"
    case post   = "POST"
    case put    = "PUT"
    case delete = "DELETE"
}

struct APIError: Error, LocalizedError {
    let statusCode: Int
    let message: String
    var errorDescription: String? { "HTTP \(statusCode): \(message)" }
}

// MARK: - Client

actor NetworkClient {
    private let base: URL
    private let session: URLSession
    private let decoder = JSONDecoder()

    init(base: URL, session: URLSession = .shared) {
        self.base    = base
        self.session = session
        decoder.keyDecodingStrategy     = .convertFromSnakeCase
        decoder.dateDecodingStrategy    = .iso8601
    }

    func fetch<T: Decodable>(_ type: T.Type, path: String, method: HTTPMethod = .get, body: (some Encodable)? = Optional<Int>.none) async throws -> T {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = method.rawValue
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        if let body {
            request.httpBody = try JSONEncoder().encode(body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        guard (200..<300).contains(http.statusCode) else {
            let msg = String(data: data, encoding: .utf8) ?? "Unknown error"
            throw APIError(statusCode: http.statusCode, message: msg)
        }
        return try decoder.decode(T.self, from: data)
    }
}
