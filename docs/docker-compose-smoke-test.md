# Isolated Docker Compose smoke test

This test starts the crawler and scraper with an isolated Compose project, a
temporary configuration, and a dedicated SQLite database. The scraper
connects to Google Chrome running visibly on the Linux host. It seeds pending X
URLs into the test database and checks that the scraper visits them. It does
not mount or modify the repository's `config.yaml`, `crawler.db`, archive, or
Docker volumes used by the normal Compose project.

Run the commands below from the repository root, in the same shell so the
temporary-directory variables and `dc` helper remain available.

## 1. Create the test configuration

```sh
SMOKE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/web-archiver-compose-smoke.XXXXXX")"
export SMOKE_DIR
export ARCHIVER_CONFIG="$SMOKE_DIR/config.yaml"

cat > "$ARCHIVER_CONFIG" <<'YAML'
archive_dir: /data/archive
db: /data/smoke.db
workers: 1
min_free_space: 0
hosts:
  - name: X
    domains:
      - x.com
      - www.x.com
    use_playwright: true
seed_urls: []
YAML
```

## 2. Start visible host Chrome

Open a separate terminal on the Linux host and start Chrome with a dedicated
test profile. This opens a visible Chrome window for manual sign-in, separate
from your everyday Chrome profile:

```sh
google-chrome \
  --remote-debugging-address=127.0.0.1 \
  --remote-debugging-port=9222 \
  --user-data-dir="$HOME/.config/web-archiver-smoke-chrome"
```

Sign in to X in that window if needed, then verify Chrome's local debugging
endpoint responds:

```sh
curl --fail http://127.0.0.1:9222/json/version
```

Keep Chrome running throughout the test. The profile and cookies persist in
`~/.config/web-archiver-smoke-chrome` after Chrome closes. Do not use your
regular Chrome profile for remote debugging.

## 3. Define an isolated Compose project

The unique project name gives this test its own named volumes and network. The
override points the crawler and scraper at `/data/smoke.db`, regardless of the
database paths used by the regular Compose stack.

```sh
cat > "$SMOKE_DIR/compose.yaml" <<'YAML'
services:
  archiver:
    command: ["--archive-dir", "/data/archive", "--db", "/data/smoke.db"]
    healthcheck:
      test: ["CMD-SHELL", "test -f /data/smoke.db"]

  scraper:
    command:
      - node
      - scraper.js
      - /data/smoke.db
      - /data/archive/json/
      - /data/visited-pages.jsonl
YAML

dc() {
  docker compose \
    -p web-archiver-smoke \
    -f docker-compose.yml \
    -f "$SMOKE_DIR/compose.yaml" \
    "$@"
}

dc config -q
dc build archiver scraper
dc up -d archiver
dc ps
```

Wait for `archiver` to report healthy. The scraper uses host networking on
Linux to connect to Chrome's loopback-only debugging endpoint. Verify
connectivity from the scraper image:

```sh
dc run --rm --no-deps scraper node -e '
fetch("http://127.0.0.1:9222/json/version")
  .then(async (response) => {
    if (!response.ok) throw new Error(`Chrome returned ${response.status}`);
    console.log((await response.json()).Browser);
  })
  .catch((error) => { console.error(error); process.exit(1); });
'
```

## 4. Insert pending X URLs

Run this after the archiver is healthy, so the database schema exists. The
transaction creates the article, URL, and frontier rows required by the
application. `use_playwright = 1` routes the rows to the browser scraper, and
the conflict clause leaves each test URL pending for this run.

```sh
dc run --rm --no-deps scraper node -e '
const Database = require("better-sqlite3");
const db = new Database("/data/smoke.db");
db.pragma("busy_timeout = 30000");

const urls = [
  "https://x.com/NASA",
  "https://x.com/SpaceX",
];

const seed = db.transaction((items) => {
  for (const url of items) {
    const domain = new URL(url).hostname;
    db.prepare("INSERT OR IGNORE INTO articles (url) VALUES (?)").run(url);
    const articleId = db.prepare(
      "SELECT id FROM articles WHERE url = ?"
    ).pluck().get(url);

    db.prepare(`
      INSERT OR IGNORE INTO urls
        (url, domain, discovered_at, article_id, use_playwright)
      VALUES (?, ?, unixepoch(), ?, 1)
    `).run(url, domain, articleId);

    const urlId = db.prepare(
      "SELECT id FROM urls WHERE url = ?"
    ).pluck().get(url);
    db.prepare(`
      INSERT INTO frontier
        (url_id, priority, depth, discovered_from, status)
      VALUES (?, 0, 0, NULL, ?)
      ON CONFLICT(url_id) DO UPDATE
      SET status = excluded.status, claimed_at = NULL
    `).run(urlId, "pending");
  }
});

seed(urls);
console.table(db.prepare(`
  SELECT u.url, u.use_playwright, f.status
  FROM urls u
  JOIN frontier f ON f.url_id = u.id
  WHERE u.url IN (?, ?)
`).all(...urls));
db.close();
'
```

Confirm both URLs show `use_playwright` as `1` and `status` as `pending`.

## 5. Run the scraper and inspect results

```sh
dc up -d scraper
dc logs -f scraper
```

The scraper takes up to 25 pending Playwright URLs in each pass. Once it logs
that it has finished processing its queue, press **Ctrl+C** to stop following
the logs; the services continue running.

Inspect the test URLs' frontier state and the visit log:

```sh
dc exec -T scraper node -e '
const fs = require("fs");
const Database = require("better-sqlite3");
const db = new Database("/data/smoke.db", { readonly: true });
console.table(db.prepare(`
  SELECT u.url, u.use_playwright, f.status
  FROM urls u
  JOIN frontier f ON f.url_id = u.id
  WHERE u.domain IN (?, ?)
`).all("x.com", "www.x.com"));
db.close();

const visits = "/data/visited-pages.jsonl";
if (fs.existsSync(visits)) {
  console.log(fs.readFileSync(visits, "utf8"));
} else {
  console.log("No visit log was written.");
}
'
```

The frontier rows should reach `complete`, and the visit log should contain
navigation/final entries for the URLs. The scraper captures matching JSON
responses from X API endpoints into `/data/archive/json/`; whether those files
are produced depends on login state and which responses X serves during the
visit. A `complete` status alone does not prove that API data was captured, so
check the scraper logs and archive directory for that part of the test.

## 6. Clean up

Stop the test project and remove only its named volumes, including its isolated
database and archive:

```sh
dc down -v
rm -r -- "$SMOKE_DIR"
unset ARCHIVER_CONFIG SMOKE_DIR
unset -f dc
```

Then close the test Chrome window from its terminal with **Ctrl+C** when you no
longer need its signed-in profile. The normal Compose project uses a different
project name and is unaffected.
