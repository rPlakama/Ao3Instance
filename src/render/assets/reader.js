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
    var byChapterLink = document.querySelector("li.chapter.bychapter a");

    var pending = null;
    var lastSentAt = 0;
    var lastSentKey = "";
    var debounceTimer = 0;
    var rafId = 0;
    var dirty = false;
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
        if (savePctInput) savePctInput.value = pct.toFixed(2);
        if (byChapterLink) {
            byChapterLink.setAttribute(
                "href",
                "/read/" + encodeURIComponent(workId) + "?chapter=" + chapterIdx
            );
        }
    }

    function captureCurrent() {
        var el = activeChapter();
        var ch = chapterNumber(el);
        var pct = percentInChapter(el);
        return {
            ch: ch,
            pct: pct,
            payload: {
                work_id: workId,
                chapter: String(ch),
                pct: pct.toFixed(2)
            }
        };
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
        if (dirty && !restoring) {
            pending = captureCurrent().payload;
            dirty = false;
        }
        if (!pending) return;
        var key = pending.chapter + ":" + pending.pct;
        if (key === lastSentKey) return;
        lastSentKey = key;
        try {
            var body = new URLSearchParams(pending);
            if (!navigator.sendBeacon || !navigator.sendBeacon("/progress/auto", body)) {
                fetch("/progress/auto", {
                    method: "POST",
                    headers: { "content-type": "application/x-www-form-urlencoded" },
                    body: body,
                    keepalive: true
                }).catch(function () {});
            }
        } catch (e) {}
    }

    function recordCurrent() {
        dirty = false;
        var current = captureCurrent();
        paint(current.ch, current.pct);
        pending = current.payload;

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
            var el = chapters[i];
            var pct = chapterResume(el);
            if (el.getAttribute("data-resume-target") === "1" || pct > 0) {
                return { el: el, pct: pct };
            }
        }
        return null;
    }

    function scrollToSaved(target) {
        var top = chapterTop(target.el);
        var span = chapterSpan(target.el);
        var y = span > 0 ? top + (span * target.pct) / 100 : top;
        var targetY = Math.max(0, Math.round(y));
        try {
            window.scrollTo({ top: targetY, left: 0, behavior: "instant" });
        } catch (e) {
            window.scrollTo(0, targetY);
        }
    }

    var resumeTarget = findResumeTarget();
    var restoring = !!resumeTarget;
    var restoreSeq = 0;

    function markInteracted() {
        userInteracted = true;
        restoring = false;
    }

    window.addEventListener("wheel", markInteracted, { passive: true, once: true });
    window.addEventListener("touchstart", markInteracted, { passive: true, once: true });
    window.addEventListener("pointerdown", markInteracted, { passive: true, once: true });
    window.addEventListener("keydown", function onKeydown(event) {
        if (
            event.key === "Shift" ||
            event.key === "Control" ||
            event.key === "Alt" ||
            event.key === "Meta"
        ) {
            return;
        }
        window.removeEventListener("keydown", onKeydown);
        markInteracted();
    }, { passive: true });

    if (resumeTarget) {
        lastSentKey = chapterNumber(resumeTarget.el) + ":" + resumeTarget.pct.toFixed(2);
        if (window.history && "scrollRestoration" in window.history) {
            try {
                window.history.scrollRestoration = "manual";
            } catch (e) {}
        }

        var applyRestore = function () {
            if (userInteracted) {
                restoring = false;
                return;
            }
            var seq = ++restoreSeq;
            restoring = true;
            scrollToSaved(resumeTarget);
            window.requestAnimationFrame(function () {
                window.requestAnimationFrame(function () {
                    if (seq === restoreSeq) {
                        restoring = false;
                    }
                });
            });
        };

        applyRestore();
        window.requestAnimationFrame(function () {
            if (!userInteracted) applyRestore();
        });
        if (document.readyState !== "complete") {
            window.addEventListener("load", function () {
                if (!userInteracted) applyRestore();
            }, { once: true });
        }
        if (
            document.fonts &&
            document.fonts.ready &&
            typeof document.fonts.ready.then === "function"
        ) {
            document.fonts.ready.then(function () {
                if (!userInteracted) applyRestore();
            });
        }

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
        dirty = true;
        if (rafId) return;
        rafId = window.requestAnimationFrame(function () {
            rafId = 0;
            if (!restoring) recordCurrent();
        });
    }, { passive: true });

    document.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "hidden") flushBeacon();
    });
    window.addEventListener("pagehide", flushBeacon);
})();
