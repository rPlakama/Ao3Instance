#!/usr/bin/env sh
# AO3Instance — drift gate for the AO3 write-API documentation.
#
# The docs under docs/ are claims about a specific otwarchive commit. This script
# asserts those claims against the local read-only clone and the docs themselves.
#
# Usage:
#   scripts/check-ao3-routes.sh            # skip (exit 0) when the clone is absent
#   scripts/check-ao3-routes.sh --require  # fail when the clone is absent
#
# The clone is gitignored, so CI normally runs without it. Run this locally before
# touching docs/ao3-write-api.md or the write client.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
clone="$root/otwarchive"
require=${1:-}

if [ ! -d "$clone" ]; then
    echo "check-ao3-routes: no clone at otwarchive/ (gitignored reference copy)" >&2
    if [ "$require" = "--require" ]; then
        echo "  clone https://github.com/otwcode/otwarchive and pin it, then re-run" >&2
        exit 1
    fi
    echo "  skipped; pass --require to make this fatal" >&2
    exit 0
fi

fail=0
ok=0

pass() { ok=$((ok + 1)); printf '  ok   %s\n' "$1"; }
fail() { fail=$((fail + 1)); printf '  FAIL %s\n' "$1"; }

# need <file> <literal> <label>
need() {
    if grep -Fq -- "$2" "$clone/$1" 2>/dev/null; then
        pass "$3"
    else
        fail "$3  ($1: missing '$2')"
    fi
}

echo "== AO3 write-API drift gate"
echo "   clone:  $clone"
echo "   commit: $(git -C "$clone" rev-parse HEAD 2>/dev/null || echo 'not a git checkout')"
echo

echo "-- routes (config/routes.rb)"
need config/routes.rb 'resources :kudos, only: [:create]' 'POST /kudos'
need config/routes.rb 'resources :subscriptions, only: [:index, :create, :destroy]' 'POST/DELETE subscriptions'
need config/routes.rb 'patch :mark_for_later' 'PATCH mark_for_later'
need config/routes.rb 'patch :mark_as_read' 'PATCH mark_as_read'
need config/routes.rb 'resources :favorite_tags, only: [:create, :destroy]' 'favorite tags'
need config/routes.rb 'resources :bookmarks' 'bookmarks'
need config/routes.rb 'resources :comments' 'comments'
need config/routes.rb 'resources :works' 'works routes (new/create)'
need config/routes.rb 'resources :chapters' 'chapter routes'
need config/routes.rb 'namespace :v2' 'api/v2 namespace (admin-only)'
need config/routes.rb "downloads/:id/:download_title.:format" 'downloads route (robots-disallowed)'
echo

echo "-- controller params"
need app/controllers/kudos_controller.rb 'params.require(:kudo).permit(:commentable_id, :commentable_type)' 'kudo params'
need app/controllers/comments_controller.rb 'params.require(:comment).permit(' 'comment params'
need app/controllers/comments_controller.rb ':comment_content' 'comment_content field'
need app/controllers/subscriptions_controller.rb 'params.require(:subscription).permit(' 'subscription params'
need app/controllers/subscriptions_controller.rb ':subscribable_type' 'subscribable_type field'
need app/controllers/bookmarks_controller.rb 'params.require(:bookmark).permit(' 'bookmark params'
need app/controllers/bookmarks_controller.rb ':bookmarker_notes' 'bookmarker_notes field'
need app/controllers/favorite_tags_controller.rb 'params.require(:favorite_tag).permit(' 'favorite tag params'
need app/controllers/works_controller.rb 'def create' 'POST /works (create work)'
need app/controllers/works_controller.rb 'params.require(:work).permit(' 'work params'
need app/controllers/works_controller.rb 'params[:post_button]' 'post_button publish switch'
need app/controllers/works_controller.rb 'def post_draft' 'post_draft publish path'
need config/routes.rb 'get :drafts' 'user works drafts route'
need app/controllers/works_controller.rb 'def drafts' 'drafts list action'
need app/controllers/works_controller.rb 'def destroy' 'work delete action'
need config/routes.rb 'resources :chapters' 'nested chapter routes'
need app/controllers/chapters_controller.rb 'def create' 'POST chapters (create)'
need app/controllers/chapters_controller.rb 'def update' 'PUT chapters (update)'
need app/controllers/chapters_controller.rb 'post_without_preview_button' 'chapter post button'
need app/controllers/chapters_controller.rb 'params.require(:chapter).permit(' 'chapter params'
need app/views/works/drafts.html.erb 'unposted_drafts_get_deleted' 'draft deletion caution'
need app/views/works/_work_blurb.html.erb 'Post Draft' 'draft blurb action'
echo

echo "-- access model"
need app/controllers/application_controller.rb 'protect_from_forgery with: :exception' 'global CSRF protection'
need app/controllers/home_controller.rb 'form_authenticity_token' 'token dispenser endpoint'
need app/controllers/application_controller.rb 'cookies[:user_credentials]' 'user_credentials cookie'
need app/controllers/application_controller.rb '/lost_cookie' 'incomplete-jar redirect'
need config/initializers/session_store.rb "_otwarchive_session" 'session cookie name'
need config/initializers/rack_attack.rb 'Rack::Attack' 'rate limiting middleware'
echo

echo "-- policy"
need public/robots.public.txt 'Disallow: /downloads/' 'robots: /downloads/ disallowed'
need public/robots.public.txt 'Disallow: /works/search?' 'robots: search disallowed'
need app/views/home/tos_faq.html.erb 'bots or scraping' 'ToS FAQ bot clause'
echo

echo "-- limits (config/config.yml)"
need config/config.yml 'RATE_LIMIT_NUMBER: 300' 'global 300/300 s'
need config/config.yml 'RATE_LIMIT_LOGIN_ATTEMPTS: 5' 'login 5/20 s'
need config/config.yml 'RATE_LIMIT_USER_COMMENTING_NUMBER: 3' 'comments 3/900 s'
need config/config.yml 'COMMENT_MAX: 10000' 'comment length 10000'
need config/config.yml 'MAX_FAVORITE_TAGS: 20' '20 favorite tags'
echo

echo "-- docs present and pin recorded"
for doc in ao3-internals.md ao3-write-api.md security-standards.md write-client-plan.md; do
    if [ -f "$root/docs/$doc" ]; then pass "docs/$doc"; else fail "docs/$doc missing"; fi
done
for adr in 0001-auth-model 0002-access-posture 0003-v1-scope 0004-artifact-source 0005-write-safety; do
    if [ -f "$root/docs/adr/$adr.md" ]; then pass "docs/adr/$adr.md"; else fail "docs/adr/$adr.md missing"; fi
done

commit=$(git -C "$clone" rev-parse HEAD 2>/dev/null || true)
if [ -n "$commit" ]; then
    if grep -Fq "$commit" "$root/docs/ao3-internals.md" 2>/dev/null; then
        pass "clone commit pinned in docs/ao3-internals.md"
    else
        fail "docs/ao3-internals.md does not record the current clone commit ($commit)"
    fi
fi

echo
echo "== $ok passed, $fail failed"
[ "$fail" -eq 0 ] || exit 1
