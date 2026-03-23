import Foundation
import XCTest

final class MoltisUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testCanAddEnvironmentVariableFromSettings() throws {
        let runtimeRoot = try makeRuntimeRoot()
        defer {
            try? FileManager.default.removeItem(at: runtimeRoot)
        }

        let configDir = runtimeRoot.appendingPathComponent("config", isDirectory: true)
        let dataDir = runtimeRoot.appendingPathComponent("data", isDirectory: true)
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)

        let app = XCUIApplication()
        app.launchEnvironment["MOLTIS_UI_TEST_SKIP_ONBOARDING"] = "1"
        app.launchEnvironment["MOLTIS_CONFIG_DIR"] = configDir.path
        app.launchEnvironment["MOLTIS_DATA_DIR"] = dataDir.path
        app.launch()

        let settingsWindow = openSettingsWindow(in: app)
        XCTAssertTrue(settingsWindow.waitForExistence(timeout: 20))

        let environmentSection = settingsWindow
            .descendants(matching: .any)
            .matching(identifier: "settings-section-environment")
            .firstMatch
        XCTAssertTrue(environmentSection.waitForExistence(timeout: 10))
        environmentSection.click()

        let envKey = "MOLTIS_UI_TEST_\(UUID().uuidString.replacingOccurrences(of: "-", with: "_"))"

        let keyField = settingsWindow.textFields["settings-env-add-key"]
        XCTAssertTrue(keyField.waitForExistence(timeout: 10))
        clearAndType(text: envKey, into: keyField)

        let valueField = settingsWindow.secureTextFields["settings-env-add-value"]
        XCTAssertTrue(valueField.waitForExistence(timeout: 10))
        clearAndType(text: "ui-test-value", into: valueField)

        let addButton = settingsWindow.buttons["settings-env-add-button"]
        XCTAssertTrue(addButton.waitForExistence(timeout: 10))
        addButton.click()

        let successMessage = settingsWindow.staticTexts["settings-env-message"]
        XCTAssertTrue(successMessage.waitForExistence(timeout: 20))

        let addedKeyLabel = settingsWindow.staticTexts[envKey]
        XCTAssertTrue(addedKeyLabel.waitForExistence(timeout: 20))
    }

    func testSettingsShortcutDoesNotOpenMultipleWindows() throws {
        let runtimeRoot = try makeRuntimeRoot()
        defer {
            try? FileManager.default.removeItem(at: runtimeRoot)
        }

        let configDir = runtimeRoot.appendingPathComponent("config", isDirectory: true)
        let dataDir = runtimeRoot.appendingPathComponent("data", isDirectory: true)
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)

        let app = XCUIApplication()
        app.launchEnvironment["MOLTIS_UI_TEST_SKIP_ONBOARDING"] = "1"
        app.launchEnvironment["MOLTIS_CONFIG_DIR"] = configDir.path
        app.launchEnvironment["MOLTIS_DATA_DIR"] = dataDir.path
        app.launch()

        let settingsWindow = openSettingsWindow(in: app)
        XCTAssertTrue(settingsWindow.waitForExistence(timeout: 20))
        assertSingleSettingsWindow(in: app, timeout: 20)

        app.typeKey(",", modifierFlags: .command)
        assertSingleSettingsWindow(in: app, timeout: 10)

        app.typeKey(",", modifierFlags: .command)
        assertSingleSettingsWindow(in: app, timeout: 10)
    }
}

private extension MoltisUITests {
    func openSettingsWindow(in app: XCUIApplication) -> XCUIElement {
        let settingsWindow = settingsWindow(in: app)
        if settingsWindow.exists {
            return settingsWindow
        }

        app.activate()
        let openSettingsButton = app.buttons["open-settings-button"]
        if openSettingsButton.waitForExistence(timeout: 5) {
            openSettingsButton.click()
        } else {
            app.typeKey(",", modifierFlags: .command)
        }
        return settingsWindow
    }

    func settingsWindows(in app: XCUIApplication) -> XCUIElementQuery {
        app.windows.containing(.any, identifier: "settings-section-environment")
    }

    func settingsWindow(in app: XCUIApplication) -> XCUIElement {
        settingsWindows(in: app).firstMatch
    }

    func assertSingleSettingsWindow(
        in app: XCUIApplication,
        timeout: TimeInterval,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let settingsWindows = settingsWindows(in: app)
        let predicate = NSPredicate(format: "count == 1")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: settingsWindows)
        let result = XCTWaiter.wait(for: [expectation], timeout: timeout)
        XCTAssertEqual(
            result,
            .completed,
            "Expected exactly one Settings window, found \(settingsWindows.count)",
            file: file,
            line: line
        )
    }

    func makeRuntimeRoot() throws -> URL {
        let root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("moltis-ui-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    func clearAndType(text: String, into element: XCUIElement) {
        element.click()
        if let existingValue = element.value as? String, !existingValue.isEmpty {
            let deleteSequence = String(
                repeating: XCUIKeyboardKey.delete.rawValue,
                count: existingValue.count
            )
            element.typeText(deleteSequence)
        }
        element.typeText(text)
    }
}
