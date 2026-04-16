// swift-tools-version:5.3

import Foundation
import PackageDescription

var sources = ["src/parser.c"]
if FileManager.default.fileExists(atPath: "src/scanner.c") {
    sources.append("src/scanner.c")
}

let package = Package(
    name: "TreeSitterHl7v2",
    products: [
        .library(name: "TreeSitterHl7v2", targets: ["TreeSitterHl7v2"]),
    ],
    dependencies: [
        .package(url: "https://github.com/tree-sitter/swift-tree-sitter", from: "0.8.0"),
    ],
    targets: [
        .target(
            name: "TreeSitterHl7v2",
            dependencies: [],
            path: ".",
            sources: sources,
            resources: [
                .copy("queries")
            ],
            publicHeadersPath: "bindings/swift",
            cSettings: [.headerSearchPath("src")]
        ),
        .testTarget(
            name: "TreeSitterHl7v2Tests",
            dependencies: [
                "SwiftTreeSitter",
                "TreeSitterHl7v2",
            ],
            path: "bindings/swift/TreeSitterHl7v2Tests"
        )
    ],
    cLanguageStandard: .c11
)
