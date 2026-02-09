// ── Chat UI ─────────────────────────────────────────────────

import { formatTokens, parseErrorMessage, sendRpc, updateCountdown } from "./helpers.js";
import * as S from "./state.js";

// Scroll chat to bottom and keep it pinned until layout settles.
// Uses a ResizeObserver to catch any late layout shifts (sidebar re-render,
// font loading, async style recalc) and re-scrolls until stable.
export function scrollChatToBottom() {
	if (!S.chatMsgBox) return;
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	var box = S.chatMsgBox;
	var observer = new ResizeObserver(() => {
		box.scrollTop = box.scrollHeight;
	});
	observer.observe(box);
	setTimeout(() => {
		observer.disconnect();
	}, 500);
}

export function chatAddMsg(cls, content, isHtml) {
	if (!S.chatMsgBox) return null;
	var welcome = document.getElementById("welcomeCard");
	if (welcome) welcome.remove();
	var el = document.createElement("div");
	el.className = `msg ${cls}`;
	if (isHtml) {
		// Safe: content is produced by renderMarkdown which escapes via esc() first,
		// then only adds our own formatting tags (pre, code, strong).
		el.innerHTML = content;
	} else {
		el.textContent = content;
	}
	S.chatMsgBox.appendChild(el);
	if (!S.chatBatchLoading) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	return el;
}

export function stripChannelPrefix(text) {
	return text.replace(/^\[Telegram(?:\s+from\s+[^\]]+)?\]\s*/, "");
}

export function appendChannelFooter(el, channel) {
	var ft = document.createElement("div");
	ft.className = "msg-channel-footer";
	var label = channel.channel_type || "channel";
	var who = channel.username ? `@${channel.username}` : channel.sender_name;
	if (who) label += ` \u00b7 ${who}`;
	if (channel.message_kind === "voice") {
		var icon = document.createElement("span");
		icon.className = "voice-icon";
		icon.setAttribute("aria-hidden", "true");
		ft.appendChild(icon);
	}

	var text = document.createElement("span");
	text.textContent = `via ${label}`;
	ft.appendChild(text);
	el.appendChild(ft);
}

export function removeThinking() {
	var el = document.getElementById("thinkingIndicator");
	if (el) el.remove();
}

export function chatAddErrorCard(err) {
	if (!S.chatMsgBox) return;
	var el = document.createElement("div");
	el.className = "msg error-card";

	var icon = document.createElement("div");
	icon.className = "error-icon";
	icon.textContent = err.icon || "\u26A0\uFE0F";
	el.appendChild(icon);

	var body = document.createElement("div");
	body.className = "error-body";

	var title = document.createElement("div");
	title.className = "error-title";
	title.textContent = err.title;
	body.appendChild(title);

	if (err.detail) {
		var detail = document.createElement("div");
		detail.className = "error-detail";
		detail.textContent = err.detail;
		body.appendChild(detail);
	}

	if (err.provider) {
		var prov = document.createElement("div");
		prov.className = "error-detail";
		prov.textContent = `Provider: ${err.provider}`;
		prov.style.marginTop = "4px";
		prov.style.opacity = "0.6";
		body.appendChild(prov);
	}

	if (err.resetsAt) {
		var countdown = document.createElement("div");
		countdown.className = "error-countdown";
		el.appendChild(body);
		el.appendChild(countdown);
		updateCountdown(countdown, err.resetsAt);
		var timer = setInterval(() => {
			if (updateCountdown(countdown, err.resetsAt)) clearInterval(timer);
		}, 1000);
	} else {
		el.appendChild(body);
	}

	S.chatMsgBox.appendChild(el);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

export function chatAddErrorMsg(message) {
	chatAddErrorCard(parseErrorMessage(message));
}

export function renderApprovalCard(requestId, command) {
	if (!S.chatMsgBox) return;
	var tpl = document.getElementById("tpl-approval-card");
	var frag = tpl.content.cloneNode(true);
	var card = frag.firstElementChild;
	card.id = `approval-${requestId}`;

	card.querySelector(".approval-cmd").textContent = command;

	var allowBtn = card.querySelector(".approval-allow");
	var denyBtn = card.querySelector(".approval-deny");
	allowBtn.onclick = () => {
		resolveApproval(requestId, "approved", command, card);
	};
	denyBtn.onclick = () => {
		resolveApproval(requestId, "denied", null, card);
	};

	var countdown = card.querySelector(".approval-countdown");
	var remaining = 120;
	var timer = setInterval(() => {
		remaining--;
		countdown.textContent = `${remaining}s`;
		if (remaining <= 0) {
			clearInterval(timer);
			card.classList.add("approval-expired");
			allowBtn.disabled = true;
			denyBtn.disabled = true;
			countdown.textContent = "expired";
		}
	}, 1000);
	countdown.textContent = `${remaining}s`;

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

export function resolveApproval(requestId, decision, command, card) {
	var params = { requestId: requestId, decision: decision };
	if (command) params.command = command;
	sendRpc("exec.approval.resolve", params).then(() => {
		card.classList.add("approval-resolved");
		card.querySelectorAll(".approval-btn").forEach((b) => {
			b.disabled = true;
		});
		var status = document.createElement("div");
		status.className = "approval-status";
		status.textContent = decision === "approved" ? "Allowed" : "Denied";
		card.appendChild(status);
	});
}

export function highlightAndScroll(msgEls, messageIndex, query) {
	var target = null;
	if (messageIndex >= 0 && messageIndex < msgEls.length && msgEls[messageIndex]) {
		target = msgEls[messageIndex];
	}
	var lowerQ = query.toLowerCase();
	if (!target || (target.textContent || "").toLowerCase().indexOf(lowerQ) === -1) {
		for (var candidate of msgEls) {
			if (candidate && (candidate.textContent || "").toLowerCase().indexOf(lowerQ) !== -1) {
				target = candidate;
				break;
			}
		}
	}
	if (!target) return;
	msgEls.forEach((el) => {
		if (el) highlightTermInElement(el, query);
	});
	target.scrollIntoView({ behavior: "smooth", block: "center" });
	target.classList.add("search-highlight-msg");
	setTimeout(() => {
		if (!S.chatMsgBox) return;
		S.chatMsgBox.querySelectorAll("mark.search-term-highlight").forEach((m) => {
			var parent = m.parentNode;
			parent.replaceChild(document.createTextNode(m.textContent), m);
			parent.normalize();
		});
		S.chatMsgBox.querySelectorAll(".search-highlight-msg").forEach((el) => {
			el.classList.remove("search-highlight-msg");
		});
	}, 5000);
}

export function highlightTermInElement(el, query) {
	var walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null, false);
	var nodes = [];
	while (walker.nextNode()) nodes.push(walker.currentNode);
	var lowerQ = query.toLowerCase();
	nodes.forEach((textNode) => {
		var text = textNode.nodeValue;
		var lowerText = text.toLowerCase();
		var idx = lowerText.indexOf(lowerQ);
		if (idx === -1) return;
		var frag = document.createDocumentFragment();
		var pos = 0;
		while (idx !== -1) {
			if (idx > pos) frag.appendChild(document.createTextNode(text.substring(pos, idx)));
			var mark = document.createElement("mark");
			mark.className = "search-term-highlight";
			mark.textContent = text.substring(idx, idx + query.length);
			frag.appendChild(mark);
			pos = idx + query.length;
			idx = lowerText.indexOf(lowerQ, pos);
		}
		if (pos < text.length) frag.appendChild(document.createTextNode(text.substring(pos)));
		textNode.parentNode.replaceChild(frag, textNode);
	});
}

export function chatAutoResize() {
	if (!S.chatInput) return;
	S.chatInput.style.height = "auto";
	S.chatInput.style.height = `${Math.min(S.chatInput.scrollHeight, 120)}px`;
}

export function updateTokenBar() {
	var bar = S.$("tokenBar");
	if (!bar) return;
	var total = S.sessionTokens.input + S.sessionTokens.output;
	if (total === 0) {
		bar.textContent = "";
		return;
	}
	var text =
		formatTokens(S.sessionTokens.input) +
		" in / " +
		formatTokens(S.sessionTokens.output) +
		" out \u00b7 " +
		formatTokens(total) +
		" tokens";
	if (S.sessionContextWindow > 0) {
		var pct = Math.max(0, 100 - Math.round((total / S.sessionContextWindow) * 100));
		text += ` \u00b7 Context left before auto-compact: ${pct}%`;
	}
	if (!S.sessionToolsEnabled) {
		text += " \u00b7 Tools: disabled";
	}
	bar.textContent = text;
}
