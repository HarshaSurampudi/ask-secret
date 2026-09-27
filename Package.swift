// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "AskSecret",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "ask-secret", targets: ["AskSecret"])],
    targets: [.executableTarget(name: "AskSecret", path: "Sources/AskSecret")]
)
