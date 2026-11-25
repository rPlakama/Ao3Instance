/* Clickable, accessible dropdowns (primary nav, user menu, login, chapter index).
   On touch/mobile devices CSS hover is disabled and clicks toggle `.open` so
   menus stay open reliably until dismissed or an item is chosen. */
(function () {
    "use strict";

    var TOGGLE_SELECTOR = "[data-dropdown-toggle]";
    var canHover = window.matchMedia &&
        window.matchMedia("(hover: hover) and (pointer: fine)").matches;

    function getDropdown(toggle) {
        return toggle.closest(".dropdown");
    }

    function setExpanded(dropdown, open) {
        if (!dropdown) return;
        dropdown.classList.toggle("open", open);
        var toggle = dropdown.querySelector(TOGGLE_SELECTOR);
        if (toggle) {
            toggle.setAttribute("aria-expanded", open ? "true" : "false");
        }
    }

    function closeAll(exceptDropdown) {
        var opened = document.querySelectorAll(".dropdown.open");
        for (var i = 0; i < opened.length; i++) {
            if (opened[i] !== exceptDropdown) {
                setExpanded(opened[i], false);
            }
        }
        var logins = document.querySelectorAll("details.login-toggle[open]");
        for (var j = 0; j < logins.length; j++) {
            if (!exceptDropdown || !logins[j].contains(exceptDropdown)) {
                logins[j].removeAttribute("open");
            }
        }
    }

    function menuItems(dropdown) {
        var menu = dropdown.querySelector(":scope > .menu");
        if (!menu) return [];
        return Array.prototype.slice.call(
            menu.querySelectorAll("a[href], button:not(:disabled), select:not(:disabled), input:not([type=hidden]):not(:disabled)")
        );
    }

    document.addEventListener("click", function (event) {
        var toggle = event.target.closest(TOGGLE_SELECTOR);
        if (toggle) {
            var dropdown = getDropdown(toggle);
            if (!dropdown) return;
            event.preventDefault();
            event.stopPropagation();
            var isOpen = dropdown.classList.contains("open");
            closeAll(isOpen ? null : dropdown);
            setExpanded(dropdown, !isOpen);
            return;
        }

        var insideMenu = event.target.closest(".dropdown > .menu");
        if (insideMenu) {
            // Let form controls inside menus (e.g. Chapter Index select) work without closing.
            if (event.target.closest("form, select, input, label, option")) {
                event.stopPropagation();
            }
            return;
        }

        if (event.target.closest("details.login-toggle")) {
            var opened = document.querySelectorAll(".dropdown.open");
            for (var i = 0; i < opened.length; i++) {
                setExpanded(opened[i], false);
            }
            return;
        }

        closeAll(null);
    });

    document.addEventListener("keydown", function (event) {
        if (event.key === "Escape") {
            var openDropdown = document.querySelector(".dropdown.open");
            var toggle = openDropdown ? openDropdown.querySelector(TOGGLE_SELECTOR) : null;
            closeAll(null);
            if (toggle) toggle.focus();
            return;
        }

        if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") {
            return;
        }

        var dropdown = event.target.closest(".dropdown");
        if (!dropdown) return;
        var toggleEl = dropdown.querySelector(TOGGLE_SELECTOR);
        if (!toggleEl) return;

        if (event.target === toggleEl && event.key === "ArrowDown") {
            event.preventDefault();
            if (!dropdown.classList.contains("open")) {
                closeAll(dropdown);
                setExpanded(dropdown, true);
            }
            var firstItems = menuItems(dropdown);
            if (firstItems.length) firstItems[0].focus();
            return;
        }

        if (!dropdown.classList.contains("open")) return;
        if (event.target.closest("select, input, textarea")) return;

        var items = menuItems(dropdown);
        if (!items.length) return;
        var idx = items.indexOf(event.target);
        if (idx === -1) return;

        event.preventDefault();
        if (event.key === "ArrowDown") {
            items[(idx + 1) % items.length].focus();
        } else if (event.key === "ArrowUp") {
            items[(idx - 1 + items.length) % items.length].focus();
        } else if (event.key === "Home") {
            items[0].focus();
        } else if (event.key === "End") {
            items[items.length - 1].focus();
        }
    });

    // On hover-capable desktops, hovering a sibling dropdown clears a click-pinned
    // neighbor so two menus never stack visually on top of each other.
    if (canHover) {
        var navDropdowns = document.querySelectorAll("#header .dropdown");
        for (var k = 0; k < navDropdowns.length; k++) {
            navDropdowns[k].addEventListener("mouseenter", function () {
                var current = this;
                var openSiblings = document.querySelectorAll("#header .dropdown.open");
                for (var m = 0; m < openSiblings.length; m++) {
                    if (openSiblings[m] !== current) {
                        setExpanded(openSiblings[m], false);
                    }
                }
            });
        }
    }
})();

/* Live AO3 tag autocomplete + removable tag chips for `.autocomplete-field`.
   Progressively enhances plain comma-separated `<input type="text">` fields
   using `/autocomplete/{kind}?term=...` with zero HTML injection. */
(function () {
    "use strict";

    var fields = document.querySelectorAll(".autocomplete-field[data-autocomplete]");
    if (!fields.length || typeof window.fetch !== "function") return;

    var clientCache = Object.create(null);

    function appendHighlighted(parent, text, query) {
        var tokens = query
            .trim()
            .split(/\s+/)
            .filter(Boolean)
            .map(function (t) {
                return t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
            });
        if (!tokens.length) {
            parent.appendChild(document.createTextNode(text));
            return;
        }
        var regex = new RegExp("(" + tokens.join("|") + ")", "gi");
        var lastIndex = 0;
        var match;
        while ((match = regex.exec(text)) !== null) {
            if (match.index > lastIndex) {
                parent.appendChild(document.createTextNode(text.slice(lastIndex, match.index)));
            }
            var strong = document.createElement("strong");
            strong.appendChild(document.createTextNode(match[0]));
            parent.appendChild(strong);
            lastIndex = match.index + match[0].length;
            if (match[0].length === 0) break;
        }
        if (lastIndex < text.length) {
            parent.appendChild(document.createTextNode(text.slice(lastIndex)));
        }
    }

    function enhanceField(container) {
        var kind = container.getAttribute("data-autocomplete") || "tag";
        var source = container.querySelector('input[type="text"]');
        if (!source) return;

        var originalId = source.id || "";
        var originalPlaceholder = source.placeholder || "";
        var initialTags = (source.value || "")
            .split(",")
            .map(function (t) { return t.trim(); })
            .filter(Boolean);

        var hidden = document.createElement("input");
        hidden.type = "hidden";
        hidden.name = source.name;
        hidden.value = initialTags.join(", ");

        var chipList = document.createElement("ul");
        chipList.className = "autocomplete-tags";

        var input = document.createElement("input");
        input.type = "text";
        input.className = "autocomplete-input";
        if (originalId) input.id = originalId;
        if (originalPlaceholder) input.placeholder = originalPlaceholder;
        input.autocomplete = "off";
        input.spellcheck = false;
        input.setAttribute("role", "combobox");
        input.setAttribute("aria-autocomplete", "list");
        input.setAttribute("aria-expanded", "false");

        var dropdown = document.createElement("div");
        dropdown.className = "autocomplete-dropdown";
        dropdown.hidden = true;
        var list = document.createElement("ul");
        list.setAttribute("role", "listbox");
        dropdown.appendChild(list);

        source.parentNode.replaceChild(hidden, source);
        container.appendChild(chipList);
        container.appendChild(input);
        container.appendChild(dropdown);

        var tags = [];
        var suggestions = [];
        var activeIndex = -1;
        var debounceTimer = null;
        var activeController = null;

        function syncHidden() {
            var pending = input.value.trim();
            var all = tags.slice();
            if (pending) {
                pending.split(",").forEach(function (part) {
                    var clean = part.trim();
                    if (clean && !hasTag(all, clean)) {
                        all.push(clean);
                    }
                });
            }
            hidden.value = all.join(", ");
        }

        function hasTag(arr, name) {
            var lower = name.toLowerCase();
            for (var i = 0; i < arr.length; i++) {
                if (arr[i].toLowerCase() === lower) return true;
            }
            return false;
        }

        function renderChips() {
            chipList.textContent = "";
            chipList.hidden = tags.length === 0;
            for (var i = 0; i < tags.length; i++) {
                (function (tagName, index) {
                    var li = document.createElement("li");
                    li.className = "added tag";
                    var span = document.createElement("span");
                    span.appendChild(document.createTextNode(tagName));
                    var btn = document.createElement("button");
                    btn.type = "button";
                    btn.className = "delete";
                    btn.setAttribute("aria-label", "Remove " + tagName);
                    btn.appendChild(document.createTextNode("\u00d7"));
                    btn.addEventListener("click", function () {
                        tags.splice(index, 1);
                        renderChips();
                        syncHidden();
                        input.focus();
                    });
                    li.appendChild(span);
                    li.appendChild(btn);
                    chipList.appendChild(li);
                })(tags[i], i);
            }
        }

        function addTag(name) {
            var clean = name.trim();
            if (!clean) return;
            if (!hasTag(tags, clean)) {
                tags.push(clean);
                renderChips();
            }
            input.value = "";
            syncHidden();
            closeDropdown();
        }

        function closeDropdown() {
            dropdown.hidden = true;
            input.setAttribute("aria-expanded", "false");
            activeIndex = -1;
            suggestions = [];
        }

        function openDropdown() {
            dropdown.hidden = false;
            input.setAttribute("aria-expanded", "true");
        }

        function highlightActive() {
            var items = list.querySelectorAll("li.autocomplete-option");
            for (var i = 0; i < items.length; i++) {
                var isCurrent = i === activeIndex;
                items[i].classList.toggle("active", isCurrent);
                items[i].setAttribute("aria-selected", isCurrent ? "true" : "false");
                if (isCurrent && items[i].scrollIntoView) {
                    items[i].scrollIntoView({ block: "nearest" });
                }
            }
        }

        function showStatus(message) {
            list.textContent = "";
            suggestions = [];
            activeIndex = -1;
            var li = document.createElement("li");
            li.className = "autocomplete-status";
            li.appendChild(document.createTextNode(message));
            list.appendChild(li);
            openDropdown();
        }

        function renderSuggestions(items, term) {
            list.textContent = "";
            suggestions = items.filter(function (item) {
                return item && typeof item.name === "string" && item.name.trim() !== "";
            });
            activeIndex = -1;
            if (!suggestions.length) {
                showStatus("No suggestions found");
                return;
            }
            for (var i = 0; i < suggestions.length; i++) {
                (function (item, idx) {
                    var li = document.createElement("li");
                    li.className = "autocomplete-option";
                    li.setAttribute("role", "option");
                    li.setAttribute("aria-selected", "false");
                    appendHighlighted(li, item.name, term);
                    li.addEventListener("mousedown", function (event) {
                        event.preventDefault();
                    });
                    li.addEventListener("click", function () {
                        addTag(item.name);
                        input.focus();
                    });
                    li.addEventListener("mouseenter", function () {
                        activeIndex = idx;
                        highlightActive();
                    });
                    list.appendChild(li);
                })(suggestions[i], i);
            }
            openDropdown();
        }

        function querySuggestions(term) {
            var cacheKey = kind + "\0" + term.toLowerCase();
            if (Object.prototype.hasOwnProperty.call(clientCache, cacheKey)) {
                renderSuggestions(clientCache[cacheKey], term);
                return;
            }
            if (activeController) {
                activeController.abort();
                activeController = null;
            }
            if (typeof window.AbortController === "function") {
                activeController = new AbortController();
            }
            if (dropdown.hidden) {
                showStatus("Searching\u2026");
            }
            var url = "/autocomplete/" + encodeURIComponent(kind) + "?term=" + encodeURIComponent(term);
            var opts = { credentials: "same-origin", headers: { Accept: "application/json" } };
            if (activeController) opts.signal = activeController.signal;

            window.fetch(url, opts)
                .then(function (resp) {
                    if (!resp.ok) throw new Error("HTTP " + resp.status);
                    return resp.json();
                })
                .then(function (data) {
                    var items = Array.isArray(data) ? data : [];
                    clientCache[cacheKey] = items;
                    if (input.value.trim() === term) {
                        renderSuggestions(items, term);
                    }
                })
                .catch(function (err) {
                    if (err && err.name === "AbortError") return;
                    if (input.value.trim() === term) {
                        showStatus("No suggestions available");
                    }
                });
        }

        initialTags.forEach(function (t) {
            if (!hasTag(tags, t)) tags.push(t);
        });
        renderChips();
        syncHidden();

        input.addEventListener("input", function () {
            var val = input.value;
            if (val.indexOf(",") !== -1) {
                var parts = val.split(",");
                for (var i = 0; i < parts.length - 1; i++) {
                    var part = parts[i].trim();
                    if (part && !hasTag(tags, part)) {
                        tags.push(part);
                    }
                }
                renderChips();
                input.value = parts[parts.length - 1].replace(/^\s+/, "");
            }
            syncHidden();

            var term = input.value.trim();
            if (debounceTimer) {
                clearTimeout(debounceTimer);
                debounceTimer = null;
            }
            if (!term) {
                if (activeController) {
                    activeController.abort();
                    activeController = null;
                }
                closeDropdown();
                return;
            }
            debounceTimer = setTimeout(function () {
                querySuggestions(term);
            }, 200);
        });

        input.addEventListener("keydown", function (event) {
            if (event.key === "Backspace" && input.value === "" && tags.length > 0) {
                tags.pop();
                renderChips();
                syncHidden();
                return;
            }
            if (event.key === "Escape") {
                if (!dropdown.hidden) {
                    event.preventDefault();
                    event.stopPropagation();
                    closeDropdown();
                }
                return;
            }
            if (dropdown.hidden || !suggestions.length) {
                return;
            }
            if (event.key === "ArrowDown") {
                event.preventDefault();
                activeIndex = (activeIndex + 1) % suggestions.length;
                highlightActive();
            } else if (event.key === "ArrowUp") {
                event.preventDefault();
                activeIndex = (activeIndex - 1 + suggestions.length) % suggestions.length;
                highlightActive();
            } else if (event.key === "Enter" && activeIndex >= 0 && activeIndex < suggestions.length) {
                event.preventDefault();
                addTag(suggestions[activeIndex].name);
            }
        });

        input.addEventListener("focus", function () {
            var term = input.value.trim();
            if (term && dropdown.hidden) {
                querySuggestions(term);
            }
        });

        document.addEventListener("click", function (event) {
            if (!container.contains(event.target)) {
                closeDropdown();
            }
        });

        var form = container.closest("form");
        if (form) {
            form.addEventListener("submit", function () {
                syncHidden();
            });
        }
    }

    for (var i = 0; i < fields.length; i++) {
        enhanceField(fields[i]);
    }
})();

/* Narrow screens: collapse the AO3-style Sort & Filter sidebar unless the
   server marked the current filter set as active. Desktop and no-JS readers
   keep the server-rendered `open` attribute (the summary is styled away).
   Crossing the breakpoint re-opens the panel so the desktop sidebar is always
   usable. */
(function () {
    "use strict";

    var panels = document.querySelectorAll("details.filters-panel");
    if (!panels.length || !window.matchMedia) {
        return;
    }
    var narrow = window.matchMedia("(max-width: 62em)");

    function collapseInactive() {
        for (var i = 0; i < panels.length; i++) {
            if (panels[i].getAttribute("data-active") !== "true") {
                panels[i].removeAttribute("open");
            }
        }
    }

    function openAll() {
        for (var i = 0; i < panels.length; i++) {
            panels[i].setAttribute("open", "");
        }
    }

    if (narrow.matches) {
        collapseInactive();
    }
    if (narrow.addEventListener) {
        narrow.addEventListener("change", function (event) {
            if (event.matches) {
                collapseInactive();
            } else {
                openAll();
            }
        });
    }
})();
