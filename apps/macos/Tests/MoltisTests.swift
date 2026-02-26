@testable import Moltis
import XCTest

final class MoltisTests: XCTestCase {
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

    func testProviderStoreLoadsKnownProviders() throws {
        let store = ProviderStore()
        store.loadKnownProviders()

        XCTAssertFalse(store.knownProviders.isEmpty)
    }
}
