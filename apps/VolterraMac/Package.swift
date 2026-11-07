// swift-tools-version: 5.9
import PackageDescription
import Foundation

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let bridgeLibDir = "\(packageDir)/Sources/Bridge"

let package = Package(
    name: "VolterraMac",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "VolterraMac", targets: ["VolterraMac"]),
        .library(name: "VolterraBridge", targets: ["VolterraBridge"])
    ],
    targets: [
        // 1. Clang System Module wrapping volterra_ffi.h & libvolterra_ffi
        .systemLibrary(
            name: "CVolterra",
            path: "Sources/Bridge"
        ),

        // 2. Swift Concurrency Bridge Library
        .target(
            name: "VolterraBridge",
            dependencies: ["CVolterra"],
            path: "Sources/Bridge",
            exclude: [
                "libvolterra_ffi.a",
                "libvolterra_ffi.dylib",
                "volterra_ffi.h",
                "module.modulemap",
                "VolterraMac-Bridging-Header.h"
            ],
            sources: [
                "VolterraBridgeTypes.swift",
                "VolterraBridge.swift"
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-L\(bridgeLibDir)",
                    "-lvolterra_ffi"
                ])
            ]
        ),

        // 3. SwiftUI Native Desktop Application
        .executableTarget(
            name: "VolterraMac",
            dependencies: ["VolterraBridge", "CVolterra"],
            path: "Sources",
            exclude: [
                "Bridge"
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-L\(bridgeLibDir)",
                    "-lvolterra_ffi",
                    "-Xlinker", "-rpath",
                    "-Xlinker", "\(bridgeLibDir)"
                ])
            ]
        ),

        // 4. Automated Unit & Integration Tests
        .testTarget(
            name: "VolterraMacTests",
            dependencies: ["VolterraMac", "VolterraBridge", "CVolterra"],
            path: "Tests/VolterraMacTests",
            linkerSettings: [
                .unsafeFlags([
                    "-L\(bridgeLibDir)",
                    "-lvolterra_ffi"
                ])
            ]
        )
    ]
)
