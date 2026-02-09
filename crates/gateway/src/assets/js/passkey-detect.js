// ── Passkey authenticator detection ──────────────────────────
//
// Extracts the AAGUID from a WebAuthn credential's authenticator data
// and maps it to a human-readable authenticator name.

// AAGUID → friendly authenticator name
// Source: https://github.com/passkeydeveloper/passkey-authenticator-aaguids
var AAGUID_NAMES = {
	"fbfc3007-154e-4ecc-8c0b-6e020557d7bd": "Apple Passwords",
	"dd4ec289-e01d-41c9-bb89-70fa845d4bf2": "iCloud Keychain (Managed)",
	"adce0002-35bc-c60a-648b-0b25f1f05503": "Chrome on Mac",
	"ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4": "Google Password Manager",
	"08987058-cadc-4b81-b6e1-30de50dcbe96": "Windows Hello",
	"9ddd1817-af5a-4672-a2b9-3e3dd95000a9": "Windows Hello",
	"6028b017-b1d4-4c02-b4b3-afcdafc96bb2": "Windows Hello",
	"bada5566-a7aa-401f-bd96-45619a55120d": "1Password",
	"d548826e-79b4-db40-a3d8-11116f7e8349": "Bitwarden",
	"531126d6-e717-415c-9320-3d9aa6981239": "Dashlane",
	"b84e4048-15dc-4dd0-8640-f4f60813c8af": "NordPass",
	"0ea242b4-43c4-4a1b-8b17-dd6d0b6baec6": "Keeper",
	"f3809540-7f14-49c1-a8b3-8f813b225541": "Enpass",
	"53414d53-554e-4700-0000-000000000000": "Samsung Pass",
	"b5397666-4885-aa6b-cebf-e52262a439a2": "Chromium Browser",
	"771b48fd-d3d4-4f74-9232-fc157ab0507a": "Edge on Mac",
	"891494da-2c90-4d31-a9cd-4eab0aed1309": "Sésame",
};

/**
 * Detect a friendly name for a newly created passkey credential.
 *
 * Tries (in order):
 * 1. AAGUID lookup from authenticator data (bytes 37–52)
 * 2. `authenticatorAttachment` hint ("This device" / "Security key")
 * 3. Generic "Passkey" fallback
 *
 * @param {PublicKeyCredential} cred - credential returned by navigator.credentials.create()
 * @returns {string} human-readable authenticator name
 */
export function detectPasskeyName(cred) {
	try {
		var authData = new Uint8Array(cred.response.getAuthenticatorData());
		// AAGUID is at bytes 37–52 (rpIdHash:32 + flags:1 + signCount:4 = 37)
		if (authData.length >= 53) {
			var hex = "";
			for (var i = 37; i < 53; i++) hex += authData[i].toString(16).padStart(2, "0");
			var uuid =
				hex.slice(0, 8) +
				"-" +
				hex.slice(8, 12) +
				"-" +
				hex.slice(12, 16) +
				"-" +
				hex.slice(16, 20) +
				"-" +
				hex.slice(20);
			// Skip all-zero AAGUID (no attestation / none attestation)
			if (uuid !== "00000000-0000-0000-0000-000000000000") {
				var name = AAGUID_NAMES[uuid];
				if (name) return name;
			}
		}
	} catch (_e) {
		// getAuthenticatorData() may not be available in all browsers
	}
	// Fallback: platform vs cross-platform
	if (cred.authenticatorAttachment === "platform") return "This device";
	if (cred.authenticatorAttachment === "cross-platform") return "Security key";
	return "Passkey";
}
