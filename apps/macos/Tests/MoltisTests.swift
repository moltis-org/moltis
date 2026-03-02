@testable import Moltis
import XCTest

final class MoltisTests: XCTestCase {
    private func localizedStringsDictionary(for localization: String) throws -> [String: String] {
        guard
            let path = Bundle.main.path(
                forResource: "Localizable",
                ofType: "strings",
                inDirectory: nil,
                forLocalization: localization
            )
        else {
            throw NSError(
                domain: "MoltisTests.Localization",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Missing \(localization).lproj/Localizable.strings"]
            )
        }

        guard let dict = NSDictionary(contentsOfFile: path) as? [String: String] else {
            throw NSError(
                domain: "MoltisTests.Localization",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "Failed to parse \(path)"]
            )
        }
        return dict
    }

    func testVersionPayloadDecodesCoreFields() throws {
        let client = MoltisClient()
        let payload = try client.version()

        XCTAssertFalse(payload.bridgeVersion.isEmpty)
        XCTAssertFalse(payload.moltisVersion.isEmpty)
        XCTAssertFalse(payload.configDir.isEmpty)
    }

    func testChatPayloadReturnsReplyAndValidation() throws {
        let client = MoltisClient()
        let payload = try client.chat(
            message: "swift test",
            configToml: "[server]\nport = \"invalid\""
        )

        // Reply is populated (either from LLM or fallback message)
        XCTAssertFalse(payload.reply.isEmpty)
        XCTAssertNotNil(payload.validation)
        XCTAssertTrue(payload.validation?.hasErrors ?? false)
    }

    func testChatStoreAppendsUserMessageAndSends() throws {
        let settings = AppSettings()
        settings.configurationToml = "[server]\nport = \"invalid\""

        let providerStore = ProviderStore()
        let store = ChatStore(settings: settings, providerStore: providerStore)
        store.draftMessage = "store integration test"
        store.sendDraftMessage()

        let selectedSession = try XCTUnwrap(store.selectedSession)

        // User message is appended synchronously before dispatch
        XCTAssertTrue(selectedSession.messages.contains(where: {
            $0.role == .user && $0.text.contains("store integration test")
        }))

        // The assistant reply arrives asynchronously via DispatchQueue.
        // Wait briefly for the background work to complete.
        let expectation = expectation(description: "chat response")
        DispatchQueue.main.asyncAfter(deadline: .now() + 8.0) {
            expectation.fulfill()
        }
        wait(for: [expectation], timeout: 10.0)

        let updatedSession = try XCTUnwrap(store.selectedSession)
        let hasAssistantOrError = updatedSession.messages.contains(where: {
            $0.role == .assistant || $0.role == .error
        })
        XCTAssertTrue(hasAssistantOrError)
    }

    func testOnboardingStatePersistsCompletion() throws {
        let suiteName = "moltis.tests.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suiteName) else {
            XCTFail("Failed to create isolated UserDefaults suite")
            return
        }

        defaults.removePersistentDomain(forName: suiteName)
        let key = "onboarding"

        let state = OnboardingState(defaults: defaults, completionKey: key)
        XCTAssertFalse(state.isCompleted)

        state.complete()
        XCTAssertTrue(state.isCompleted)

        let reloaded = OnboardingState(defaults: defaults, completionKey: key)
        XCTAssertTrue(reloaded.isCompleted)
    }

    func testFrenchLocalizationIncludesSettingsTitle() throws {
        guard
            let frPath = Bundle.main.path(forResource: "fr", ofType: "lproj"),
            let frBundle = Bundle(path: frPath)
        else {
            XCTFail("French localization bundle is missing")
            return
        }

        let value = frBundle.localizedString(
            forKey: "Settings",
            value: nil,
            table: "Localizable"
        )
        XCTAssertEqual(value, "Réglages")
    }

    func testEnglishAndFrenchLocalizationKeysAreInSync() throws {
        let en = try localizedStringsDictionary(for: "en")
        let fr = try localizedStringsDictionary(for: "fr")

        XCTAssertFalse(en.isEmpty)
        XCTAssertFalse(fr.isEmpty)

        let missingInFrench = Set(en.keys).subtracting(fr.keys).sorted()
        let missingInEnglish = Set(fr.keys).subtracting(en.keys).sorted()

        XCTAssertTrue(
            missingInFrench.isEmpty,
            "Missing French localization keys: \(missingInFrench.joined(separator: ", "))"
        )
        XCTAssertTrue(
            missingInEnglish.isEmpty,
            "Unexpected French-only localization keys: \(missingInEnglish.joined(separator: ", "))"
        )
    }

    // MARK: - Provider bridge tests

    func testKnownProvidersReturnsNonEmptyArray() throws {
        let client = MoltisClient()
        let providers = try client.knownProviders()

        XCTAssertFalse(providers.isEmpty)
        let first = try XCTUnwrap(providers.first)
        XCTAssertFalse(first.name.isEmpty)
        XCTAssertFalse(first.displayName.isEmpty)
        XCTAssertFalse(first.authType.isEmpty)
    }

    func testDetectProvidersReturnsArray() throws {
        let client = MoltisClient()
        // Should return an array (may be empty if no providers are configured)
        let sources = try client.detectProviders()
        // Just verify it doesn't throw and returns a valid array
        _ = sources
    }

    func testListModelsReturnsArray() throws {
        let client = MoltisClient()
        let models = try client.listModels()
        // Just verify it doesn't throw and returns a valid array
        _ = models
    }

    func testRefreshRegistrySucceeds() throws {
        let client = MoltisClient()
        // Should not throw
        try client.refreshRegistry()
    }

    func testListEnvVarsReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.listEnvVars()
        XCTAssertFalse(payload.vaultStatus.isEmpty)
        _ = payload.envVars
    }

    func testMemoryStatusReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.memoryStatus()
        _ = payload.available
        _ = payload.totalFiles
        _ = payload.totalChunks
    }

    func testMemoryConfigGetReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.memoryConfigGet()
        XCTAssertFalse(payload.backend.isEmpty)
        XCTAssertFalse(payload.citations.isEmpty)
    }

    func testMemoryQmdStatusReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.memoryQmdStatus()
        _ = payload.featureEnabled
        _ = payload.available
    }

    func testSandboxStatusReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.sandboxStatus()
        XCTAssertFalse(payload.backend.isEmpty)
        XCTAssertFalse(payload.os.isEmpty)
        XCTAssertFalse(payload.defaultImage.isEmpty)
    }

    func testSandboxSharedHomeReturnsPayload() throws {
        let client = MoltisClient()
        let payload = try client.sandboxGetSharedHome()
        XCTAssertFalse(payload.mode.isEmpty)
        XCTAssertFalse(payload.path.isEmpty)
    }

    func testAuthStatusReturnsPayload() throws {
        let client = MoltisClient()
        let status = try client.authStatus()
        _ = status.authDisabled
        _ = status.hasPassword
        _ = status.hasPasskeys
        _ = status.setupComplete
    }

    func testAuthPasskeyListReturnsPayload() throws {
        let client = MoltisClient()
        let passkeys = try client.authListPasskeys()
        _ = passkeys
    }

    func testAuthPasswordChangeRejectsShortPassword() {
        let client = MoltisClient()
        XCTAssertThrowsError(
            try client.authPasswordChange(currentPassword: nil, newPassword: "short")
        ) { error in
            guard case let MoltisClientError.bridgeError(code, _) = error else {
                XCTFail("Expected bridgeError for short password")
                return
            }
            XCTAssertEqual(code, "AUTH_PASSWORD_TOO_SHORT")
        }
    }

    func testProviderStoreLoadsKnownProviders() throws {
        let store = ProviderStore()
        store.loadKnownProviders()

        XCTAssertFalse(store.knownProviders.isEmpty)
    }
}
