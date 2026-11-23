/* Local reading-position tracker. The chapter iframe posts scroll percentages;
   this script saves them and updates the reader's progress bar. */
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

    function requestProgress() {
        frames.forEach(function (frame) {
            try {
                frame.contentWindow.postMessage({type: "ao3i:request"}, "*");
            } catch (e) {}
        });
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

    window.addEventListener("message", function (event) {
        if (frames.indexOf(event.source) === -1) return;
        var data = event.data;
        if (!data || data.type !== "ao3i:progress") return;
        var chapter = Number(data.chapter);
        var pct = Number(data.pct);
        if (!Number.isInteger(chapter) || chapter < 1 || !Number.isFinite(pct)) return;
        schedule(chapter, pct);
    });

    requestProgress();
    window.setTimeout(requestProgress, 500);
    window.setTimeout(requestProgress, 1500);

    window.addEventListener("pagehide", function () {
        if (!pending) return;
        try {
            navigator.sendBeacon("/progress/auto", new URLSearchParams(pending));
        } catch (e) {}
    });
})();
