//
// LocalizationTests.swift
// VolterraMacTests
//

import XCTest
import SwiftUI
@testable import VolterraMac

@MainActor
final class LocalizationTests: XCTestCase {
    func testEighteenSupportedLanguagesExist() {
        XCTAssertEqual(AppLanguage.allCases.count, 18, "VOLTERRA must support exactly 18 languages")
    }

    func testAllEighteenLanguagesHaveCompleteTranslations() {
        for language in AppLanguage.allCases {
            guard let dict = LocalizationManager.translations[language] else {
                XCTFail("Missing translation dictionary for language: \(language.rawValue)")
                continue
            }

            for key in L10nKey.allCases {
                let value = dict[key]
                XCTAssertNotNil(value, "Missing key \(key.rawValue) in language \(language.rawValue)")
                XCTAssertFalse(value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true,
                               "Empty translation for key \(key.rawValue) in language \(language.rawValue)")
            }
        }
    }

    func testLanguageSwitchingAndRTLBehavior() {
        let manager = LocalizationManager(initialLanguage: .turkish)
        XCTAssertEqual(manager.currentLanguage, .turkish)
        XCTAssertEqual(manager.t(.startTransfer), "Aktarımı Başlat")
        XCTAssertEqual(manager.currentLanguage.layoutDirection, .leftToRight)

        manager.currentLanguage = .english
        XCTAssertEqual(manager.t(.startTransfer), "Start Transfer")
        XCTAssertEqual(manager.currentLanguage.layoutDirection, .leftToRight)

        manager.currentLanguage = .japanese
        XCTAssertEqual(manager.t(.startTransfer), "転送を開始")

        manager.currentLanguage = .arabic
        XCTAssertEqual(manager.t(.startTransfer), "بدء النقل")
        XCTAssertTrue(manager.currentLanguage.isRightToLeft)
        XCTAssertEqual(manager.currentLanguage.layoutDirection, .rightToLeft)
    }
}
