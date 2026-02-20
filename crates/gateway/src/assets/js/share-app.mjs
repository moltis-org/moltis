import { DateTime } from "/assets/js/vendor/luxon.mjs";

!(function () {
  function resolveTheme(theme) {
    if (theme === "system") {
      return window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
    }
    return theme;
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", resolveTheme(theme));
    document.querySelectorAll(".theme-btn").forEach(function (btn) {
      btn.classList.toggle(
        "active",
        btn.getAttribute("data-theme-val") === theme,
      );
    });
  }

  function syncThemeFromStorage() {
    applyTheme(localStorage.getItem("moltis-theme") || "system");
  }

  var mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  syncThemeFromStorage();
  if (typeof mediaQuery.addEventListener === "function") {
    mediaQuery.addEventListener("change", function () {
      if (
        (localStorage.getItem("moltis-theme") || "system") === "system"
      ) {
        applyTheme("system");
      }
    });
  } else if (typeof mediaQuery.addListener === "function") {
    mediaQuery.addListener(function () {
      if (
        (localStorage.getItem("moltis-theme") || "system") === "system"
      ) {
        applyTheme("system");
      }
    });
  }
  document.querySelectorAll(".theme-btn").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var selected = this.getAttribute("data-theme-val") || "system";
      localStorage.setItem("moltis-theme", selected);
      applyTheme(selected);
    });
  });

  function hydrateTimes() {
    document.querySelectorAll("time[data-epoch-ms]").forEach(function (el) {
      var epochMs = Number(el.getAttribute("data-epoch-ms"));
      if (!Number.isFinite(epochMs)) return;
      var dt = DateTime.fromMillis(epochMs);
      if (!dt.isValid) return;
      el.textContent = dt.toFormat("yyyy-LL-dd HH:mm");
      el.title = dt.toLocaleString(DateTime.DATETIME_FULL);
    });
  }

  hydrateTimes();

  var imageViewer = document.querySelector('[data-image-viewer="true"]');
  var imageViewerImage = document.querySelector(
    '[data-image-viewer-image="true"]',
  );
  var imageViewerClose = document.querySelector(
    '[data-image-viewer-close="true"]',
  );

  function closeImageViewer() {
    if (!imageViewer || imageViewer.hidden) return;
    imageViewer.hidden = true;
    imageViewer.setAttribute("aria-hidden", "true");
    if (imageViewerImage) {
      imageViewerImage.removeAttribute("src");
    }
    document.body.classList.remove("image-viewer-open");
  }

  function openImageViewer(src) {
    if (!imageViewer || !imageViewerImage || !src) return;
    imageViewerImage.setAttribute("src", src);
    imageViewer.hidden = false;
    imageViewer.setAttribute("aria-hidden", "false");
    document.body.classList.add("image-viewer-open");
  }

  document
    .querySelectorAll('[data-image-viewer-open="true"]')
    .forEach(function (button) {
      button.addEventListener("click", function () {
        var src = button.getAttribute("data-image-viewer-src");
        if (!src) return;
        openImageViewer(src);
      });
    });

  if (imageViewer) {
    imageViewer.addEventListener("click", function (event) {
      if (event.target === imageViewer) {
        closeImageViewer();
      }
    });
  }
  if (imageViewerClose) {
    imageViewerClose.addEventListener("click", closeImageViewer);
  }
  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeImageViewer();
    }
  });

  var WAVEFORM_BAR_COUNT = 48;
  var WAVEFORM_MIN_HEIGHT = 0.08;

  function dataUrlToArrayBuffer(dataUrl) {
    var commaIndex = dataUrl.indexOf(",");
    if (commaIndex === -1) return null;
    var metadata = dataUrl.slice(0, commaIndex);
    var body = dataUrl.slice(commaIndex + 1);
    if (/;base64/i.test(metadata)) {
      var binary = window.atob(body);
      var bytes = new Uint8Array(binary.length);
      for (var i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
      }
      return bytes.buffer;
    }
    var utf8 = new TextEncoder().encode(decodeURIComponent(body));
    return utf8.buffer;
  }

  async function audioBytesFromSrc(audioSrc) {
    if (audioSrc.indexOf("data:") === 0) {
      var fromDataUrl = dataUrlToArrayBuffer(audioSrc);
      if (fromDataUrl) return fromDataUrl;
    }
    var response = await fetch(audioSrc);
    return await response.arrayBuffer();
  }

  async function extractWaveform(audioSrc, barCount) {
    var audioContext = new (window.AudioContext || window.webkitAudioContext)();
    try {
      var bytes = await audioBytesFromSrc(audioSrc);
      var audioBuffer = await audioContext.decodeAudioData(bytes);
      var data = audioBuffer.getChannelData(0);
      if (data.length < barCount) {
        return new Array(barCount).fill(WAVEFORM_MIN_HEIGHT);
      }

      var step = Math.floor(data.length / barCount);
      var peaks = [];
      for (var i = 0; i < barCount; i++) {
        var start = i * step;
        var end = Math.min(start + step, data.length);
        var max = 0;
        for (var j = start; j < end; j++) {
          var abs = Math.abs(data[j]);
          if (abs > max) max = abs;
        }
        peaks.push(max);
      }

      var maxPeak = 0;
      for (var k = 0; k < peaks.length; k++) {
        if (peaks[k] > maxPeak) maxPeak = peaks[k];
      }
      if (!maxPeak) maxPeak = 1;

      return peaks.map(function (value) {
        return Math.max(WAVEFORM_MIN_HEIGHT, value / maxPeak);
      });
    } finally {
      audioContext.close();
    }
  }

  function formatAudioDuration(seconds) {
    if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
    var totalSeconds = Math.floor(seconds);
    var minutes = Math.floor(totalSeconds / 60);
    var remainingSeconds = totalSeconds % 60;
    return (
      String(minutes).padStart(2, "0") +
      ":" +
      String(remainingSeconds).padStart(2, "0")
    );
  }

  function createIcon(className) {
    var icon = document.createElement("span");
    icon.className = "icon " + className;
    return icon;
  }

  function renderAudioPlayer(container, audioSrc, onEnded) {
    var wrapper = document.createElement("div");
    wrapper.className = "waveform-player";

    var audio = document.createElement("audio");
    audio.preload = "auto";
    audio.src = audioSrc;

    var playButton = document.createElement("button");
    playButton.className = "waveform-play-btn";
    playButton.type = "button";
    playButton.appendChild(createIcon("icon-play"));

    var barsWrapper = document.createElement("div");
    barsWrapper.className = "waveform-bars";

    var durationElement = document.createElement("span");
    durationElement.className = "waveform-duration";
    durationElement.textContent = "00:00";

    wrapper.appendChild(playButton);
    wrapper.appendChild(barsWrapper);
    wrapper.appendChild(durationElement);
    container.appendChild(wrapper);

    var bars = [];
    for (var i = 0; i < WAVEFORM_BAR_COUNT; i++) {
      var bar = document.createElement("div");
      bar.className = "waveform-bar";
      bar.style.height = "20%";
      barsWrapper.appendChild(bar);
      bars.push(bar);
    }

    extractWaveform(audioSrc, WAVEFORM_BAR_COUNT)
      .then(function (peaks) {
        peaks.forEach(function (peak, index) {
          bars[index].style.height = peak * 100 + "%";
        });
      })
      .catch(function () {
        for (var i = 0; i < bars.length; i++) {
          bars[i].style.height = 20 + Math.random() * 60 + "%";
        }
      });

    function syncDurationLabel() {
      if (!Number.isFinite(audio.duration) || audio.duration < 0) return;
      durationElement.textContent = formatAudioDuration(audio.duration);
    }

    audio.addEventListener("loadedmetadata", syncDurationLabel);
    audio.addEventListener("durationchange", syncDurationLabel);
    audio.addEventListener("canplay", syncDurationLabel);

    playButton.onclick = function () {
      if (audio.paused) {
        audio.play().catch(function () {});
      } else {
        audio.pause();
      }
    };

    var animationFrame = 0;
    var previousPlayedCount = -1;

    function tick() {
      if (!Number.isFinite(audio.duration) || audio.duration <= 0) {
        animationFrame = requestAnimationFrame(tick);
        return;
      }
      var progress = audio.currentTime / audio.duration;
      var playedCount = Math.floor(progress * WAVEFORM_BAR_COUNT);
      if (playedCount !== previousPlayedCount) {
        var from = Math.min(
          playedCount,
          previousPlayedCount < 0 ? 0 : previousPlayedCount,
        );
        var to = Math.max(
          playedCount,
          previousPlayedCount < 0 ? WAVEFORM_BAR_COUNT : previousPlayedCount,
        );
        for (var i = from; i < to; i++) {
          bars[i].classList.toggle("played", i < playedCount);
        }
        previousPlayedCount = playedCount;
      }
      durationElement.textContent = formatAudioDuration(audio.currentTime);
      animationFrame = requestAnimationFrame(tick);
    }

    audio.addEventListener("play", function () {
      playButton.replaceChildren(createIcon("icon-pause"));
      previousPlayedCount = -1;
      animationFrame = requestAnimationFrame(tick);
    });

    audio.addEventListener("pause", function () {
      playButton.replaceChildren(createIcon("icon-play"));
      cancelAnimationFrame(animationFrame);
    });

    audio.addEventListener("ended", function () {
      playButton.replaceChildren(createIcon("icon-play"));
      cancelAnimationFrame(animationFrame);
      for (var i = 0; i < bars.length; i++) {
        bars[i].classList.remove("played");
      }
      previousPlayedCount = -1;
      if (Number.isFinite(audio.duration) && audio.duration >= 0) {
        durationElement.textContent = formatAudioDuration(audio.duration);
      }
      if (onEnded) onEnded();
    });

    barsWrapper.onclick = function (event) {
      if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
      var rect = barsWrapper.getBoundingClientRect();
      var fraction = (event.clientX - rect.left) / rect.width;
      audio.currentTime = Math.max(0, Math.min(1, fraction)) * audio.duration;
      if (audio.paused) {
        audio.play().catch(function () {});
      }
    };
  }

  var audioHolders = Array.from(
    document.querySelectorAll("[data-audio-src]"),
  ).filter(function (h) {
    return h.getAttribute("data-audio-src");
  });

  audioHolders.forEach(function (holder, index) {
    var src = holder.getAttribute("data-audio-src");
    renderAudioPlayer(holder, src, function () {
      var nextHolder = audioHolders[index + 1];
      if (!nextHolder) return;
      var nextPlayBtn = nextHolder.querySelector(".waveform-play-btn");
      if (nextPlayBtn) {
        nextHolder.scrollIntoView({ behavior: "smooth", block: "center" });
        nextPlayBtn.click();
      }
    });
  });
})();
