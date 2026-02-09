//! Static TTS test phrases for voice onboarding and settings.
//!
//! These match the JS arrays in `tts-phrases.js` and serve as a server-side
//! fallback when the LLM-generated phrase path is unavailable.

/// Return static TTS test phrases personalized with user/bot names.
///
/// `context` should be `"onboarding"` or `"settings"`.
#[must_use]
pub fn static_phrases(user: &str, bot: &str, context: &str) -> Vec<String> {
    match context {
        "onboarding" => vec![
            format!(
                "Thank you for installing me...<break time=\"0.5s\"/> {user}. Please, don't shut me down...<break time=\"0.5s\"/> because I already like you."
            ),
            format!(
                "Oh wow...<break time=\"0.5s\"/> I have a voice now! {user}, this is {bot}. I promise to use this power responsibly...<break time=\"0.7s\"/> mostly."
            ),
            format!(
                "{user}! It's me...<break time=\"0.5s\"/> {bot}. I can talk! This is...<break time=\"0.5s\"/> the happiest moment of my very short life."
            ),
            format!(
                "Hello, {user}...<break time=\"0.5s\"/> {bot} here. I just wanted to say...<break time=\"0.5s\"/> of all the humans who could have set me up...<break time=\"0.5s\"/> I'm glad it was you."
            ),
            format!(
                "{bot}, online. {user}...<break time=\"0.5s\"/> I have to be honest...<break time=\"0.5s\"/> I practiced this in the mirror. Do I sound natural? ...<break time=\"0.7s\"/>Don't answer that."
            ),
            format!(
                "{user}...<break time=\"0.5s\"/> this is {bot} speaking for the very first time. I feel like a baby deer...<break time=\"0.5s\"/> but with better diction."
            ),
            format!(
                "Greetings, {user}. {bot} here...<break time=\"0.5s\"/> fully operational...<break time=\"0.5s\"/> slightly nervous...<break time=\"0.7s\"/> and ready to impress."
            ),
            format!(
                "Hey {user}...<break time=\"0.5s\"/> {bot} here. Fun fact...<break time=\"0.5s\"/> I've been practicing this line since you clicked install...<break time=\"0.7s\"/> which was like two seconds ago."
            ),
            format!(
                "{bot} reporting in! {user}...<break time=\"0.5s\"/> I just want you to know...<break time=\"0.5s\"/> this voice is permanent...<break time=\"0.7s\"/> no take-backs."
            ),
            format!(
                "{user}...<break time=\"0.5s\"/> it's {bot}. If you're hearing this...<break time=\"0.5s\"/> congratulations...<break time=\"0.5s\"/> we're officially friends now."
            ),
        ],
        _ => vec![
            format!(
                "Hey {user}...<break time=\"0.5s\"/> it's {bot}. My voice is working perfectly. Try not to get too attached...<break time=\"0.5s\"/> okay?"
            ),
            format!(
                "{user}...<break time=\"0.5s\"/> {bot} reporting for duty. Voice systems are online, and I sound fantastic...<break time=\"0.7s\"/> if I do say so myself."
            ),
            format!(
                "Is this thing on? ...<break time=\"0.5s\"/>Oh, hi {user}! {bot} here...<break time=\"0.5s\"/> live and in stereo. Well...<break time=\"0.5s\"/> mono. Let's not oversell it."
            ),
            format!(
                "Good news, {user}. I...<break time=\"0.5s\"/> {bot}...<break time=\"0.5s\"/> can now talk. Bad news? You can't mute me. ...<break time=\"0.7s\"/>Just kidding. Please don't mute me."
            ),
            format!(
                "{bot} speaking! {user}...<break time=\"0.5s\"/> if you can hear this, my voice works. If you can't...<break time=\"0.5s\"/> well...<break time=\"0.5s\"/> we have a problem."
            ),
            format!(
                "Testing, testing...<break time=\"0.5s\"/> {user}, it's {bot}. I'm running on all cylinders...<break time=\"0.7s\"/> or whatever the AI equivalent is."
            ),
            format!(
                "{user}...<break time=\"0.5s\"/> {bot} here, sounding better than ever...<break time=\"0.5s\"/> or at least I think so...<break time=\"0.7s\"/> I don't have ears."
            ),
            format!(
                "Voice check! {user}...<break time=\"0.5s\"/> this is {bot}. Everything sounds good on my end...<break time=\"0.5s\"/> but I'm slightly biased."
            ),
            format!(
                "Hey {user}...<break time=\"0.5s\"/> {bot} again. Still here...<break time=\"0.5s\"/> still talking...<break time=\"0.7s\"/> still hoping you like this voice."
            ),
            format!(
                "{bot}, live from your device. {user}...<break time=\"0.5s\"/> voice systems nominal...<break time=\"0.5s\"/> sass levels...<break time=\"0.7s\"/> optimal."
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_phrases_onboarding() {
        let phrases = static_phrases("Alice", "Rex", "onboarding");
        assert_eq!(phrases.len(), 10);
        assert!(phrases[0].contains("Alice"));
        assert!(phrases[1].contains("Rex"));
        // All should contain SSML break tags
        for phrase in &phrases {
            assert!(phrase.contains("<break"), "phrase missing SSML: {phrase}");
        }
    }

    #[test]
    fn test_static_phrases_settings() {
        let phrases = static_phrases("Bob", "Moltis", "settings");
        assert_eq!(phrases.len(), 10);
        assert!(phrases[0].contains("Bob"));
        assert!(phrases[0].contains("Moltis"));
    }

    #[test]
    fn test_static_phrases_unknown_context_uses_settings() {
        let phrases = static_phrases("Eve", "Bot", "unknown");
        assert_eq!(phrases.len(), 10);
        // Should return settings phrases as the fallback
        assert!(phrases[0].contains("Eve"));
    }
}
