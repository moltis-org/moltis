import CoreLocation
import Foundation
import os

@MainActor
final class SettingsStore: ObservableObject {
    @Published var showToolCalls: Bool {
        didSet { UserDefaults.standard.set(showToolCalls, forKey: "showToolCalls") }
    }
    @Published var enableLiveActivities: Bool {
        didSet { UserDefaults.standard.set(enableLiveActivities, forKey: "enableLiveActivities") }
    }
    @Published var autoReconnect: Bool {
        didSet { UserDefaults.standard.set(autoReconnect, forKey: "autoReconnect") }
    }

    init() {
        let defaults = UserDefaults.standard
        defaults.register(defaults: [
            "showToolCalls": true,
            "enableLiveActivities": true,
            "autoReconnect": true,
        ])
        self.showToolCalls = defaults.bool(forKey: "showToolCalls")
        self.enableLiveActivities = defaults.bool(forKey: "enableLiveActivities")
        self.autoReconnect = defaults.bool(forKey: "autoReconnect")
    }
}

@MainActor
final class LocationSharingStore: NSObject, ObservableObject {
    @Published var isEnabled: Bool {
        didSet {
            UserDefaults.standard.set(isEnabled, forKey: Self.enabledKey)
            reevaluateTracking()
        }
    }
    @Published private(set) var authorizationStatus: CLAuthorizationStatus
    @Published private(set) var lastSentAt: Date?
    @Published private(set) var isTracking = false
    @Published private(set) var lastError: String?

    private static let enabledKey = "shareLocationWithMoltis"
    private let logger = Logger(subsystem: "org.moltis.ios", category: "location")
    private let locationManager = CLLocationManager()

    private weak var connectionStore: ConnectionStore?
    private var appIsActive = true
    private var lastSentLocation: CLLocation?
    private var sendTask: Task<Void, Never>?

    private let minSendInterval: TimeInterval = 15
    private let minSendDistanceMeters: CLLocationDistance = 40

    override init() {
        let defaults = UserDefaults.standard
        if defaults.object(forKey: Self.enabledKey) == nil {
            defaults.set(false, forKey: Self.enabledKey)
        }
        self.isEnabled = defaults.bool(forKey: Self.enabledKey)
        self.authorizationStatus = locationManager.authorizationStatus
        super.init()

        locationManager.delegate = self
        locationManager.desiredAccuracy = kCLLocationAccuracyHundredMeters
        locationManager.distanceFilter = 25

        reevaluateTracking()
    }

    func configure(connectionStore: ConnectionStore) {
        self.connectionStore = connectionStore
        reevaluateTracking()
    }

    func setAppIsActive(_ isActive: Bool) {
        appIsActive = isActive
        reevaluateTracking()
    }

    func handleConnectionStateChange() {
        reevaluateTracking()
    }

    var authorizationDescription: String {
        switch authorizationStatus {
        case .authorizedAlways:
            return "Always Allowed"
        case .authorizedWhenInUse:
            return "Allowed While Using App"
        case .notDetermined:
            return "Permission Not Requested"
        case .restricted:
            return "Restricted"
        case .denied:
            return "Denied"
        @unknown default:
            return "Unknown"
        }
    }

    private var hasPermission: Bool {
        switch authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse:
            return true
        default:
            return false
        }
    }

    private var canTrack: Bool {
        guard isEnabled, appIsActive, hasPermission else { return false }
        guard let connectionStore else { return false }
        return connectionStore.state.isConnected
    }

    private func reevaluateTracking() {
        authorizationStatus = locationManager.authorizationStatus

        guard isEnabled else {
            lastError = nil
            stopTracking()
            return
        }

        switch authorizationStatus {
        case .notDetermined:
            locationManager.requestWhenInUseAuthorization()
            stopTracking()
            return
        case .denied, .restricted:
            lastError = "Location access is off. Enable it in iOS Settings."
            stopTracking()
            return
        case .authorizedAlways, .authorizedWhenInUse:
            break
        @unknown default:
            lastError = "Unknown location permission status."
            stopTracking()
            return
        }

        lastError = nil

        guard canTrack else {
            stopTracking()
            return
        }

        if !isTracking {
            locationManager.startUpdatingLocation()
            isTracking = true
        }
    }

    private func stopTracking() {
        if isTracking {
            locationManager.stopUpdatingLocation()
            isTracking = false
        }
        sendTask?.cancel()
        sendTask = nil
    }

    private func handleLocationUpdate(_ location: CLLocation) {
        guard canTrack else { return }
        guard location.horizontalAccuracy >= 0 else { return }

        if let lastSentLocation, let lastSentAt {
            let moved = location.distance(from: lastSentLocation)
            let elapsed = Date().timeIntervalSince(lastSentAt)
            if moved < minSendDistanceMeters && elapsed < minSendInterval {
                return
            }
        }

        sendTask?.cancel()
        sendTask = Task { [weak self] in
            await self?.sendLocation(location)
        }
    }

    private func sendLocation(_ location: CLLocation) async {
        guard canTrack else { return }
        guard let connectionStore else { return }

        do {
            let ok = try await connectionStore.graphqlClient.updateUserLocation(
                latitude: location.coordinate.latitude,
                longitude: location.coordinate.longitude
            )
            guard ok else {
                lastError = "Location update was rejected by server."
                return
            }
            lastError = nil
            lastSentAt = Date()
            lastSentLocation = location
        } catch {
            lastError = error.localizedDescription
            logger.error("Location update failed: \(error.localizedDescription, privacy: .public)")
        }
    }
}

extension LocationSharingStore: CLLocationManagerDelegate {
    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.authorizationStatus = manager.authorizationStatus
            self.reevaluateTracking()
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let latest = locations.last else { return }
        Task { @MainActor [weak self] in
            self?.handleLocationUpdate(latest)
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.lastError = error.localizedDescription
            self.logger.error("Location manager error: \(error.localizedDescription, privacy: .public)")
        }
    }
}
