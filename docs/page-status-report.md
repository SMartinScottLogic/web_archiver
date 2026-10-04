# Page and processing status report

This report describes the status values currently written by the application
and the situations that cause each transition. It documents the status model in
the source code; it does not report the live status of individual database
rows. The application stores these values as plain text, without database
constraints restricting them to a fixed set.

There are three related but distinct workflows:

1. `frontier.status` tracks URLs/pages to crawl. It is shared by the normal
   HTTP crawler and the Playwright scraper.
2. `json_queue.status` tracks JSON response files captured by the scraper and
   subsequently scanned for links.
3. `emails.status` tracks archived email files that are scanned for links.

The JSON and email queues produce or augment frontier work, but their
`processed` state is not the same as a frontier URL's `complete` state.

## URL/page frontier: `frontier.status`

The `frontier` row is keyed by `url_id`. Its default status is `pending`.
Normal HTTP workers claim only rows whose associated URL has
`use_playwright = 0`; the scraper claims only `use_playwright = 1` rows.

| Status | Meaning in the code |
| --- | --- |
| `pending` | Eligible for the appropriate dispatcher: the HTTP frontier manager or Playwright scraper. |
| `in_progress` | Claimed for a fetch/visit attempt. It does not guarantee that a network request or navigation completed. |
| `complete` | The code's completion marker. For HTTP work this follows archive/media persistence; for Playwright work it follows a visit attempt. It is not proof of a successful HTTP response or captured API data. |
| `skipped` | The frontier manager rejected the URL under its crawl checks (for example, invalid/non-HTTP URL, disallowed domain, inactive host, depth or robots policy). |
| `failed` | Explicitly assigned in two paths: an HTTP task unexpectedly routed to the normal poller despite requiring Playwright, or a request error containing the `ENHANCE_YOUR_CALM` / excessive-load marker. Other fetch errors are not assigned this status. |

### Frontier transitions

| Source status | Target status | Situation causing the transition |
| --- | --- | --- |
| No row | `pending` | A URL is first enqueued from seeds, discovered links, email content, or JSON content. `INSERT ... ON CONFLICT` preserves an existing row's status; it does not re-pend an existing URL just because it is discovered again. |
| `pending` | `in_progress` | `FrontierDb::claim_next` atomically claims a non-Playwright URL for HTTP work, or the scraper sets a Playwright URL to `in_progress` immediately before opening its browser tab. |
| `in_progress` | `pending` | At crawler startup, `FrontierManager::new` resets all in-progress frontier rows. At each scraper pass, `resetQueue` resets in-progress Playwright rows. `reset_all` also sets every frontier row to pending when the configured full-reset option is enabled. |
| `complete` | `pending` | The dispatcher re-queues stale completed HTTP URLs when `latest_fetch_time` is at or before the configured refetch cutoff. This requeue is explicitly limited to `use_playwright = 0`. |
| `failed` | `pending` | The frontier dispatcher re-queues all failed URLs regardless of fetch age or `use_playwright`; the appropriate HTTP or Playwright dispatcher claims them afterward. |
| `in_progress` | `skipped` | After claim, the frontier manager's crawl-policy check returns no crawl decision, so the URL is marked skipped instead of being sent to a worker. |
| `in_progress` | `failed` | The frontier manager detects an invalid Playwright task in the normal HTTP poller. |
| Any current status for the article's URLs | `failed` | A request error contains the excessive-load marker; `mark_article` applies the status to all frontier rows associated with that article. |
| Any current status for the article's URLs | `complete` | An HTML article is finalized after its archive JSON is written; `mark_article` applies completion to all URLs associated with that article. A non-HTML response is marked complete after its media file is written. |
| `in_progress` | `complete` | The Playwright loop calls `setStatus(..., 'complete')` after its navigation attempt. It catches a `page.goto` error, logs it, and still closes the tab and marks the URL complete. |
| Any status | `pending` | If the configured full reset is enabled, `FrontierDb::reset_all` resets all frontier rows. |

The general HTTP error branch logs the failure but does not mark the URL
`failed` or return it to `pending`. Such a row can therefore remain
`in_progress` until a crawler restart resets in-progress frontier rows. The
normal HTTP fetch path also does not call `error_for_status`; HTML `FetchedPage`
metadata currently uses status code `200`. Do not interpret `complete` as
evidence that the origin returned 2xx.

`skipped` is not automatically retried. `failed` rows are reset to `pending`
by the frontier requeue regardless of age or fetch type. Both statuses can
also be reset by the configured full reset, which indiscriminately sets every
frontier row to `pending`.

## Captured JSON files: `json_queue.status`

The Playwright scraper inserts a `json_queue` row with the default status
`pending` after it saves a relevant JSON response. The Rust JSON poller claims
these rows, parses the file, discovers/enqueues URLs, then marks the queue row
processed.

| Status | Meaning in the code |
| --- | --- |
| `pending` | A saved JSON file is waiting for the JSON poller. |
| `in_progress` | The poller has claimed the file and is parsing it / extracting links. |
| `processed` | JSON parsing and link processing returned successfully and the row was marked processed. |

| Source status | Target status | Situation causing the transition |
| --- | --- | --- |
| No row | `pending` | The scraper successfully saves a relevant JSON response and inserts its file path and depth. |
| `pending` | `in_progress` | `JsonDb::claim_pending` selects the next pending file and updates its row in the same transaction. |
| `in_progress` | `processed` | The JSON file parses, links are examined and eligible links are enqueued, and the poller sets the row to processed. A valid JSON file with no eligible links is still processed. |
| `in_progress` | `pending` | `JsonDb::reset` resets in-progress JSON rows at application startup. |

There is no explicit JSON-queue `failed` status. If opening/parsing a file or
enqueuing links returns an error, the poller logs it and leaves the row
`in_progress`; the startup reset makes it eligible again on a later
application start. `processed` means this processing path returned success; it
does not mean that the file contained any eligible links.

## Archived email files: `emails.status`

The IMAP flow writes each fetched message to the email archive and enqueues its
file in the `emails` table. The schema default is `pending`.

| Status | Meaning in the code |
| --- | --- |
| `pending` | An archived email file is waiting for the mailbox poller. |
| `in_progress` | `EmailDb::next_email` claimed the row for processing. |
| `processed` | Email processing reached the status update at the end of `MailboxPoller::process`. |

| Source status | Target status | Situation causing the transition |
| --- | --- | --- |
| No row | `pending` | `EmailDb::enqueue_email` inserts a message file after the IMAP fetch writes it to disk; the schema default supplies the status. |
| `pending` | `in_progress` | `EmailDb::next_email` selects the next pending row and updates it in a transaction. |
| `in_progress` | `processed` | The file is read and the processor reaches its final status update. A MIME parse failure is ignored by the current `if let Ok(...)` branch and can still end in `processed`; URL discovery is not guaranteed. |

### Email recovery caveat

The application calls `EmailDb::reset()` at startup. However, the current
implementation's SQL updates `frontier` rows, not `emails` rows. Consequently
it does **not** change an email's `in_progress` status to `pending`. An email
processing error that leaves a row in progress can strand it; `next_email`
will not select it again. The helper can also reset in-progress frontier rows
as a side effect. There is no email `failed` status or other automatic retry
transition in the current code.

## Code locations

Use these implementation points as the evidence for this report:

- Frontier schema and defaults: `web_archiver/src/frontier/db/schema.rs`
- Frontier insertion, claiming, requeue, reset, and status updates:
  `web_archiver/src/frontier/db/frontier.rs`
- Frontier policy skips and dispatch: `web_archiver/src/frontier/frontier_manager.rs`
- HTTP fetch and media completion/error handling:
  `web_archiver/src/fetcher/worker.rs`
- HTML article archive finalization:
  `web_archiver/src/extractor/router.rs`
- Playwright queue claiming, response capture, and visit status updates:
  `playwright_scraper/scraper.js`
- JSON response processing and queue recovery: `web_archiver/src/json/mod.rs`
- Email queue processing and status updates: `web_archiver/src/mail/mod.rs`
- Email file ingestion: `web_archiver/src/mail/single.rs`
- Startup wiring and reset calls: `web_archiver/src/system.rs`

## Regenerating this report with an LLM

From the repository root, give an LLM with workspace access this prompt:

```text
Regenerate docs/page-status-report.md from the current source code. Trace every
status column and every status write/read in the crawler frontier, Playwright
scraper, JSON processing queue, and email processing queue. For each status,
explain its meaning and list every source-status -> target-status transition,
including the exact condition or event that causes it. Distinguish implemented
transitions from intended but ineffective recovery behavior. Check startup,
error, retry, and reset paths; do not infer guarantees from status names.
Include relevant source file paths, clearly call out mismatches or stranded
states, and distinguish queue-item completion from successful network or
content capture. Keep the report self-contained and update the README link if
the report location changes. Do not change application code.
```

After regeneration, review every claim against its cited status write and
caller; status behavior can change independently of this document.
