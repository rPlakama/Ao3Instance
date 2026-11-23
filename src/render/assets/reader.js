/* Local reading-position tracker.
   The chapter iframe is same-origin but script-free (sandbox="allow-same-origin"),
   so this parent script can observe its scroll position directly. */
(function () {
    "use strict";
    var frames = Array.prototype.slice.call(
        document.querySelectorAll("iframe.chapter[data-work-id]")
    );
    if (!frames.length) return;

    var workId = frames[0].getAttribute("data-work-id");
    var total = parseInt(frames[0].getAttribute("data-total-chapters"), 10) || frames.length;
    var bar = document.querySelector(".reading-progress progress");
    var label = document.querySelector(".reading-progress .reading-pct");
    var pending = null;
    var lastSent = 0;
    var lastPct = -1;

    function overall(chapter, pct) {
        var value = (((chapter - 1) + pct / 100) / Math.max(1, total)) * 100;
        return Math.max(0, Math.min(100, Math.round(value)));
    }

    function paint(chapter, pct) {
        var value = overall(chapter, pct);
        if (bar) bar.value = value;
        if (label) label.textContent = value + "% read";
    }

    function send(body) {
        lastSent = Date.now();
        try {
            fetch("/progress/auto", {
                method: "POST",
                headers: {"content-type": "application/x-www-form-urlencoded"},
                body: new URLSearchParams(body),
                keepalive: true
            }).catch(function () {});
        } catch (e) {}
    }

    function schedule(chapter, pct) {
        paint(chapter, pct);
        pending = {
            work_id: workId,
            chapter: String(chapter),
            pct: String(Math.max(0, Math.min(100, Math.round(pct))))
        };
        if (Date.now() - lastSent > 2000 || Math.abs(pct - lastPct) >= 1) {
            lastPct = pct;
            send(pending);
        }
    }

    function setup(frame) {
        var win = frame.contentWindow;
        if (!win) return;
        var chapter = parseInt(frame.getAttribute("data-chapter"), 10);
        if (!Number.isInteger(chapter) || chapter < 1) return;
        var resume = Number(frame.getAttribute("data-resume")) || 0;

        function maxScroll() {
            var doc = frame.contentDocument;
            if (!doc) return 0;
            return Math.max(0, doc.documentElement.scrollHeight - win.innerHeight);
        }

        function percent() {
            var m = maxScroll();
            if (m <= 0) return 100;
            return Math.min(100, Math.max(0, win.scrollY / m * 100));
        }

        function update() { schedule(chapter, percent()); }

        function loaded() {
            var doc = frame.contentDocument;
            return !!(doc && doc.readyState === "complete" && doc.location &&
                doc.location.pathname.indexOf("/chapter/") !== -1);
        }

        function restore() {
            window.setTimeout(function () {
                if (!loaded()) return;
                if (resume > 0) {
                    var m = maxScroll();
                    if (m > 0) win.scrollTo(0, m * resume / 100);
                }
                update();
            }, 120);
        }

        var raf = 0;
        win.addEventListener("scroll", function () {
            if (raf) return;
            raf = window.requestAnimationFrame(function () { raf = 0; update(); });
        }, {passive: true});

        var restored = false;
        function restoreOnce() {
            if (restored) return;
            restored = true;
            restore();
        }

        if (loaded()) {
            restoreOnce();
        } else {
            frame.addEventListener("load", restoreOnce, {once: true});
            // The iframe may have finished before this script ran; poll briefly.
            var attempts = 0;
            var poll = window.setInterval(function () {
                attempts += 1;
                if (loaded()) {
                    window.clearInterval(poll);
                    restoreOnce();
                } else if (attempts >= 50) {
                    window.clearInterval(poll);
                }
            }, 100);
        }
    }

    frames.forEach(setup);

    window.addEventListener("pagehide", function () {
        if (!pending) return;
        try {
            navigator.sendBeacon("/progress/auto", new URLSearchParams(pending));
        } catch (e) {}
    });
})();
