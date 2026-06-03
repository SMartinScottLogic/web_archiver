const fs = require("fs-extra");
const path = require("path");
const { chromium } = require("playwright");
const Database = require("better-sqlite3");

const args = process.argv.slice(2);

if (args.length != 3) {
  console.log(`
    Usage:
      node scraper.js <queue_db> <output_dir> <visited_pages.jsonl>

    Example:
      node scraper.js ./queue.db ./twitter_archive ./visited-pages.jsonl
  `);

  process.exit(1);
}
const QUEUE_DB = path.resolve(args[0]);
const OUTPUT_DIR = path.resolve(args[1]);
const VISITS_FILE = path.resolve(args[2]);

// Open (or create) the database file
const db = new Database(QUEUE_DB);

//
// 1. Create schema
//
db.exec(`
CREATE TABLE IF NOT EXISTS json_queue (
        id INTEGER PRIMARY KEY,
  path TEXT,
  depth INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending'
);
`);

const CDP_ENDPOINT = "http://localhost:9222";

async function appendVisitLog(entry) {
  // TODO Add to sqlite DB as a processed page
  
  const line = JSON.stringify({
    timestamp: new Date().toISOString(),
    ...entry,
  }) + "\n";

  await fs.appendFile(VISITS_FILE, line, "utf8");
}

async function resetQueue() {
  const stmt = db.prepare(`
    UPDATE frontier
    SET
      status = 'pending',
      claimed_at = NULL
    WHERE
      status = 'in_progress'
      AND url_id IN (
        SELECT id
        FROM urls
        WHERE use_playwright = 1
      )
  `);

  const result = stmt.run();
  console.log(`Reset ${result.changes} rows`);
}

async function loadQueue() {
const allJobs = db.prepare(`
  SELECT * 
  FROM urls u JOIN frontier f ON u.id = f.url_id 
  WHERE u.use_playwright=1 
  AND status = 'pending'
`).all();

//console.log("Current jobs:");
//console.table(allJobs);
return allJobs;
}

async function setStatus(url_id, status) {
      // Update URL status
    const updateJob = db.prepare(
      `UPDATE frontier 
      SET status = ? 
      WHERE url_id = ?`);

updateJob.run(status, url_id);
}

function safeFilename(url) {
  return url
    .replace(/https?:\/\//, "")
    .replace(/[^a-z0-9]/gi, "_")
    .toLowerCase()
    .slice(0, 150);
}

async function saveResponse(response, depth) {
  try {
    const url = response.url();
    const headers = response.headers();

    const contentType = headers["content-type"] || "";

    // Only keep JSON responses
    if (!contentType.includes("application/json")) return;

    // Optional stronger filtering for X/Twitter APIs
    const isRelevant =
      url.includes("x.com") ||
      url.includes("twitter.com") ||
      url.includes("/i/api/") ||
      url.includes("/graphql");

    if (!isRelevant) return;

    const data = await response.json().catch(() => null);
    if (!data) return;

    const filename = `${Date.now()}_${safeFilename(url)}.json`;
    const filePath = path.join(OUTPUT_DIR, filename);

    await fs.writeJson(
      filePath,
      {
        capturedAt: new Date().toISOString(),
        url,
        headers,
        data,
      },
      { spaces: 2 }
    );

    // Insert into DB
    const insertJob = db.prepare(`
      INSERT OR IGNORE INTO json_queue (path, depth)
      VALUES (?, ?)
    `);

    insertJob.run(filePath, depth);

    console.log("Saved:", filename);
  } catch (err) {
    console.error("failed to saveResponse", err);
    // Ignore malformed JSON / non-readable responses
  }
}

async function main() {
  await fs.ensureDir(OUTPUT_DIR);

  await resetQueue();

  const queue = await loadQueue();

  console.log("Connecting to existing Chrome via CDP...");

  const browser = await chromium.connectOverCDP(CDP_ENDPOINT);

  // Reuse existing logged-in Chrome profile/context
  const context =
    browser.contexts()[0] || (await browser.newContext());

  console.log("Connected.");
  console.log(`Loaded ${queue.length} URLs from queue.\n`);

  var count = 0;
  for (const {url, url_id, depth} of queue) {
    count += 1;
    console.log("Visiting: " + url + " (" + count + "/" + queue.length + ")");

    await setStatus(url_id, 'in_progress');
    const page = await context.newPage();
    
    const visitedUrls = new Set();

    function record(url, type) {
      if (!url || visitedUrls.has(`${type}:${url}`)) return;
      
      visitedUrls.add(`${type}:${url}`);
      console.log(`[${type}]`, url);
      
      appendVisitLog({
        type,
        url,
      }).catch(() => {});
    }
    // Attach listener per page
    page.on("response", async (response) => {
      await saveResponse(response, depth);
      
      const status = response.status();
      const url = response.url();
      
      if ([301, 302, 303, 307, 308].includes(status)) {
        record(url, "redirect");
      }
    });

    page.on("framenavigated", (frame) => {
      if (frame === page.mainFrame()) {
        record(frame.url(), "navigation");
      }
    });

    try {
      await page.goto(url, {
        waitUntil: "domcontentloaded",
        timeout: 60000,
      });

      record(page.url(), "final");

      // Allow background GraphQL/API calls to fire
      await page.waitForTimeout(4000);
    } catch (err) {
      console.error("Failed:", url, err.message);
    }

    // Close only this tab, keep Chrome alive
    await page.close();

    await setStatus(url_id, 'complete');

    console.log("Closed tab.\n");
  }

  console.log("Done processing queue.");
  await browser.close();
  
  console.log("Disconnected from Chrome.");
  process.exit(0);
}

main().catch(console.error);