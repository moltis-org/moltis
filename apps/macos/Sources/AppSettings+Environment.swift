import Foundation

extension AppSettings {
    func loadEnvironmentVariables() {
        do {
            let result = try client.listEnvVars()
            envVars = result.envVars.map { entry in
                EnvVarItem(
                    id: entry.id,
                    key: entry.key,
                    updatedAt: entry.updatedAt,
                    encrypted: entry.encrypted
                )
            }
            environmentVaultStatus = result.vaultStatus
            envError = nil
        } catch {
            envError = error.localizedDescription
            logSettingsError("load environment variables", error)
        }
    }

    func addEnvironmentVariable() {
        envError = nil
        envMessage = nil
        let key = newEnvKey.trimmingCharacters(in: .whitespacesAndNewlines)
        if key.isEmpty {
            envError = "Key is required."
            return
        }
        if !isValidEnvironmentVariableKey(key) {
            envError = "Key must contain only letters, digits, and underscores."
            return
        }

        environmentBusy = true
        do {
            try client.setEnvVar(key: key, value: newEnvValue)
            newEnvKey = ""
            newEnvValue = ""
            envMessage = "Variable saved."
            loadEnvironmentVariables()
        } catch {
            envError = error.localizedDescription
            logSettingsError("save environment variable", error)
        }
        environmentBusy = false
    }

    func startEnvironmentVariableUpdate(id: Int64) {
        updatingEnvVarId = id
        updatingEnvValue = ""
    }

    func cancelEnvironmentVariableUpdate() {
        updatingEnvVarId = nil
        updatingEnvValue = ""
    }

    func confirmEnvironmentVariableUpdate(key: String) {
        envError = nil
        envMessage = nil
        environmentBusy = true
        do {
            try client.setEnvVar(key: key, value: updatingEnvValue)
            cancelEnvironmentVariableUpdate()
            envMessage = "Variable saved."
            loadEnvironmentVariables()
        } catch {
            envError = error.localizedDescription
            logSettingsError("update environment variable", error)
        }
        environmentBusy = false
    }

    func deleteEnvironmentVariable(id: Int64) {
        envError = nil
        envMessage = nil
        do {
            try client.deleteEnvVar(id: id)
            if updatingEnvVarId == id {
                cancelEnvironmentVariableUpdate()
            }
            loadEnvironmentVariables()
        } catch {
            envError = error.localizedDescription
            logSettingsError("delete environment variable", error)
        }
    }

    func isValidEnvironmentVariableKey(_ key: String) -> Bool {
        key.allSatisfy { char in
            char.isLetter || char.isNumber || char == "_"
        }
    }
}
