(function () {
    var toast = document.getElementById('toast');
    var toastTimer;

    function showToast(message) {
        if (!toast) return;
        toast.textContent = message;
        toast.classList.remove('opacity-0', 'pointer-events-none');
        toast.classList.add('opacity-100');
        clearTimeout(toastTimer);
        toastTimer = setTimeout(function () {
            toast.classList.remove('opacity-100');
            toast.classList.add('opacity-0', 'pointer-events-none');
        }, 2000);
    }

    function fallbackCopyText(text) {
        return new Promise(function (resolve, reject) {
            var input = document.createElement('textarea');
            input.value = text;
            input.setAttribute('readonly', '');
            input.style.position = 'fixed';
            input.style.opacity = '0';
            document.body.appendChild(input);
            input.select();
            try {
                document.execCommand('copy') ? resolve() : reject();
            } catch (error) {
                reject(error);
            } finally {
                input.remove();
            }
        });
    }

    function copyText(text) {
        if (!navigator.clipboard || !navigator.clipboard.writeText) return fallbackCopyText(text);
        return navigator.clipboard.writeText(text).catch(function () {
            return fallbackCopyText(text);
        });
    }

    document.querySelectorAll('button[data-copy-text]').forEach(function (button) {
        button.addEventListener('click', function () {
            copyText(button.dataset.copyText).then(function () {
                var icon = button.querySelector('[data-copy-icon]');
                var check = button.querySelector('[data-copy-check]');
                var originalLabel = button.getAttribute('aria-label');
                if (icon) icon.classList.add('hidden');
                if (check) check.classList.remove('hidden');
                button.setAttribute('aria-label', button.dataset.copyMessage);
                showToast(button.dataset.copyMessage);
                setTimeout(function () {
                    if (icon) icon.classList.remove('hidden');
                    if (check) check.classList.add('hidden');
                    button.setAttribute('aria-label', originalLabel);
                }, 2000);
            }).catch(function () {});
        });
    });

    window.moltisClipboard = { copyText: copyText, showToast: showToast };
})();
