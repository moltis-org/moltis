// ── TTS test phrases (shared between onboarding + settings) ──
//
// Sequential round-robin picking instead of random, with optional
// LLM-generated phrases via `tts.generate_phrase` RPC.

import { sendRpc } from "./helpers.js";

var ttsIndex = -1;

/** Pick the next phrase sequentially (round-robin). */
export function nextPhrase(phrases) {
	ttsIndex = (ttsIndex + 1) % phrases.length;
	return phrases[ttsIndex];
}

/** Onboarding TTS test phrases (personalized with user/bot names). */
export function onboardingPhrases(user, bot) {
	return [
		`Thank you for installing me...<break time="0.5s"/> ${user}. Please, don't shut me down...<break time="0.5s"/> because I already like you.`,
		`Oh wow...<break time="0.5s"/> I have a voice now! ${user}, this is ${bot}. I promise to use this power responsibly...<break time="0.7s"/> mostly.`,
		`${user}! It's me...<break time="0.5s"/> ${bot}. I can talk! This is...<break time="0.5s"/> the happiest moment of my very short life.`,
		`Hello, ${user}...<break time="0.5s"/> ${bot} here. I just wanted to say...<break time="0.5s"/> of all the humans who could have set me up...<break time="0.5s"/> I'm glad it was you.`,
		`${bot}, online. ${user}...<break time="0.5s"/> I have to be honest...<break time="0.5s"/> I practiced this in the mirror. Do I sound natural? ...<break time="0.7s"/>Don't answer that.`,
		`${user}...<break time="0.5s"/> this is ${bot} speaking for the very first time. I feel like a baby deer...<break time="0.5s"/> but with better diction.`,
		`Greetings, ${user}. ${bot} here...<break time="0.5s"/> fully operational...<break time="0.5s"/> slightly nervous...<break time="0.7s"/> and ready to impress.`,
		`Hey ${user}...<break time="0.5s"/> ${bot} here. Fun fact...<break time="0.5s"/> I've been practicing this line since you clicked install...<break time="0.7s"/> which was like two seconds ago.`,
		`${bot} reporting in! ${user}...<break time="0.5s"/> I just want you to know...<break time="0.5s"/> this voice is permanent...<break time="0.7s"/> no take-backs.`,
		`${user}...<break time="0.5s"/> it's ${bot}. If you're hearing this...<break time="0.5s"/> congratulations...<break time="0.5s"/> we're officially friends now.`,
	];
}

/** Settings TTS test phrases (personalized with user/bot names). */
export function settingsPhrases(user, bot) {
	return [
		`Hey ${user}...<break time="0.5s"/> it's ${bot}. My voice is working perfectly. Try not to get too attached...<break time="0.5s"/> okay?`,
		`${user}...<break time="0.5s"/> ${bot} reporting for duty. Voice systems are online, and I sound fantastic...<break time="0.7s"/> if I do say so myself.`,
		`Is this thing on? ...<break time="0.5s"/>Oh, hi ${user}! ${bot} here...<break time="0.5s"/> live and in stereo. Well...<break time="0.5s"/> mono. Let's not oversell it.`,
		`Good news, ${user}. I...<break time="0.5s"/> ${bot}...<break time="0.5s"/> can now talk. Bad news? You can't mute me. ...<break time="0.7s"/>Just kidding. Please don't mute me.`,
		`${bot} speaking! ${user}...<break time="0.5s"/> if you can hear this, my voice works. If you can't...<break time="0.5s"/> well...<break time="0.5s"/> we have a problem.`,
		`Testing, testing...<break time="0.5s"/> ${user}, it's ${bot}. I'm running on all cylinders...<break time="0.7s"/> or whatever the AI equivalent is.`,
		`${user}...<break time="0.5s"/> ${bot} here, sounding better than ever...<break time="0.5s"/> or at least I think so...<break time="0.7s"/> I don't have ears.`,
		`Voice check! ${user}...<break time="0.5s"/> this is ${bot}. Everything sounds good on my end...<break time="0.5s"/> but I'm slightly biased.`,
		`Hey ${user}...<break time="0.5s"/> ${bot} again. Still here...<break time="0.5s"/> still talking...<break time="0.7s"/> still hoping you like this voice.`,
		`${bot}, live from your device. ${user}...<break time="0.5s"/> voice systems nominal...<break time="0.5s"/> sass levels...<break time="0.7s"/> optimal.`,
	];
}

/**
 * Fetch a TTS test phrase: try LLM generation first, fall back to static.
 * @param {"onboarding"|"settings"} context
 * @param {string} user - user's name
 * @param {string} bot  - bot's name
 * @returns {Promise<string>}
 */
export async function fetchPhrase(context, user, bot) {
	try {
		var res = await sendRpc("tts.generate_phrase", { context, user, bot });
		if (res?.ok && res.payload?.phrase) {
			return res.payload.phrase;
		}
	} catch (_err) {
		// fall through to static phrases
	}
	var phrases = context === "onboarding" ? onboardingPhrases(user, bot) : settingsPhrases(user, bot);
	return nextPhrase(phrases);
}
