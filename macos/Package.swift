// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "AskSecretUI",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "ask-secret-ui", targets: ["AskSecretUI"])],
    targets: [.executableTarget(name: "AskSecretUI", path: "Sources/AskSecretUI")]
)
