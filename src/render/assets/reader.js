/* Local reading-position tracker for inline AO3-style chapters.
   Tracks the natural window scroll position across single-chapter and
   Entire Work views and restores the exact saved scroll position on return. */
(function () {
    "use strict";

    var chapters = Array.prototype.slice.call(
        document.querySelectorAll("#chapters .chapter[data-work-id][data-chapter]")
    );
    if (!chapters.length) return;

    var workId = chapters[0].getAttribute("data-work-id");
    var total = parseInt(chapters[0].getAttribute("data-total-chapters"), 10) || chapters.length;
    var label = document.querySelector(".reading-progress .reading-pct");
    var saveChapterInput = document.querySelector("form.save-place input[name='chapter']");
    var savePctInput = document.querySelector("form.save-place input[name='pct']");

    var pending = null;
    var lastSentAt = 0;
    var lastSentKey = "";
    var debounceTimer = 0;
    var rafId = 0;
    var userInteracted = false;

    function clampPct(value) {
        if (!Number.isFinite(value)) return 0;
        return Math.max(0, Math.min(100, value));
    }

    function chapterNumber(el) {
        return parseInt(el.getAttribute("data-chapter"), 10) || 1;
    }

    function chapterResume(el) {
        var exact = Number(el.getAttribute("data-resume-exact"));
        if (Number.isFinite(exact) && exact > 0) return clampPct(exact);
        var coarse = Number(el.getAttribute("data-resume"));
        if (Number.isFinite(coarse) && coarse > 0) return clampPct(coarse);
        return 0;
    }

    function chapterTop(el) {
        return el.getBoundingClientRect().top + window.scrollY;
    }

    function chapterSpan(el) {
        return Math.max(0, el.offsetHeight - window.innerHeight);
    }

    function percentInChapter(el) {
        var top = chapterTop(el);
        var height = el.offsetHeight;
        var span = chapterSpan(el);
        if (span <= 0) {
            return (window.scrollY + window.innerHeight >= top + height - 8) ? 100 : 0;
        }
        return clampPct(((window.scrollY - top) / span) * 100);
    }

    function activeChapter() {
        if (chapters.length === 1) return chapters[0];
        var probe = window.scrollY + window.innerHeight * 0.25;
        var chosen = chapters[0];
        for (var i = 0; i < chapters.length; i++) {
            if (chapterTop(chapters[i]) <= probe) {
                chosen = chapters[i];
            } else {
                break;
            }
        }
        return chosen;
    }

    function overallPercent(chapterIdx, pct) {
        var pos = 0;
        for (var i = 0; i < chapters.length; i++) {
            if (chapterNumber(chapters[i]) === chapterIdx) {
                pos = i;
                break;
            }
        }
        var ordinal = chapters.length === 1 ? chapterIdx : pos + 1;
        var value = (((ordinal - 1) + pct / 100) / Math.max(1, total)) * 100;
        return Math.max(0, Math.min(100, Math.round(value)));
    }

    function paint(chapterIdx, pct) {
        var overall = overallPercent(chapterIdx, pct);
        if (label) label.textContent = overall + "% read";
        if (saveChapterInput) saveChapterInput.value = String(chapterIdx);
        if (savePctInput) savePctInput.value = String(Math.round(pct));
    }

    function send(payload) {
        var key = payload.chapter + ":" + payload.pct;
        if (key === lastSentKey) return;
        lastSentKey = key;
        lastSentAt = Date.now();
        try {
            fetch("/progress/auto", {
                method: "POST",
                headers: { "content-type": "application/x-www-form-urlencoded" },
                body: new URLSearchParams(payload),
                keepalive: true
            }).catch(function () {});
        } catch (e) {}
    }

    function flushBeacon() {
        if (!pending) return;
        var key = pending.chapter + ":" + pending.pct;
        if (key === lastSentKey) return;
        lastSentKey = key;
        try {
            navigator.sendBeacon("/progress/auto", new URLSearchParams(pending));
        } catch (e) {}
    }

    function recordCurrent() {
        var el = activeChapter();
        var ch = chapterNumber(el);
        var pct = percentInChapter(el);
        paint(ch, pct);

        var formatted = pct.toFixed(2);
        pending = {
            work_id: workId,
            chapter: String(ch),
            pct: formatted
        };

        var now = Date.now();
        if (now - lastSentAt >= 1500) {
            send(pending);
        }
        if (debounceTimer) window.clearTimeout(debounceTimer);
        debounceTimer = window.setTimeout(function () {
            debounceTimer = 0;
            if (pending) send(pending);
        }, 700);
    }

    function findResumeTarget() {
        if (window.location.hash && window.location.hash !== "#top") return null;
        for (var i = 0; i < chapters.length; i++) {
            var pct = chapterResume(chapters[i]);
            if (pct > 0) {
                return { el: chapters[i], pct: pct };
            }
        }
        return null;
    }

    function scrollToSaved(target) {
        var top = chapterTop(target.el);
        var span = chapterSpan(target.el);
        var y = span > 0 ? top + (span * target.pct) / 100 : top;
        window.scrollTo(0, Math.max(0, Math.round(y)));
    }

    var resumeTarget = findResumeTarget();
    var restoring = !!resumeTarget;

    function markInteracted() {
        userInteracted = true;
    }

    window.addEventListener("wheel", markInteracted, { passive: true, once: true });
    window.addEventListener("touchstart", markInteracted, { passive: true, once: true });
    window.addEventListener("keydown", markInteracted, { passive: true, once: true });

    if (resumeTarget) {
        var applyRestore = function () {
            if (userInteracted) {
                restoring = false;
                return;
            }
            scrollToSaved(resumeTarget);
        };

        window.requestAnimationFrame(function () {
            applyRestore();
            window.requestAnimationFrame(function () {
                if (!userInteracted) applyRestore();
                restoring = false;
            });
        });

        // Re-anchor if chapter images finish loading before the reader starts scrolling.
        var images = document.querySelectorAll("#chapters img");
        for (var i = 0; i < images.length; i++) {
            if (!images[i].complete) {
                images[i].addEventListener("load", function () {
                    if (!userInteracted) applyRestore();
                }, { once: true });
            }
        }
    }

    window.addEventListener("scroll", function () {
        if (restoring) return;
        if (rafId) return;
        rafId = window.requestAnimationFrame(function () {
            rafId = 0;
            recordCurrent();
        });
    }, { passive: true });

    document.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "hidden") flushBeacon();
    });
    window.addEventListener("pagehide", flushBeacon);
})();
