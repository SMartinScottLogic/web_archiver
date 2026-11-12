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
| `complete` | The code's completion marker. For ordinary HTTP article work it follows archive/media persistence; a fetched pagination URL omitted from the current page-1 refresh, or successful pages from a discarded incomplete snapshot, can also be marked complete without changing the archive. For Playwright work it follows a visit attempt. It is not proof that content was included in the current archive or that the origin returned 2xx. |
| `skipped` | The frontier manager rejected the URL under its crawl checks (for example, invalid/non-HTTP URL, disallowed domain, inactive host, depth or robots policy), or configured host-level `exclude_paths`. Excluded URLs are recorded as skipped when enqueued and are not dispatched to either fetcher. |
| `failed` | A retryable HTTP fetch/save/routing failure awaiting its persisted retry time. It is retried at most five total claimed attempts per retry cycle, with 1, 2, 4, then 8 minutes between retry attempts. Retryable failures include network errors, save errors, HTTP 5xx, 408, and 429. |
| `failed_terminal` | A non-retryable HTTP failure (including other HTTP 4xx or malformed requests), or a retryable failure after the fifth attempt in the cycle. It is not automatically retried; a later page-1 refresh that rediscovers the URL starts a new bounded attempt cycle. |

### Frontier transitions

| Source status | Target status | Situation causing the transition |
| --- | --- | --- |
| No row | `pending` | A URL that is not path-excluded is first enqueued from seeds, discovered links, email content, or JSON content. `INSERT ... ON CONFLICT` preserves an existing row's status; it does not re-pend an existing URL just because it is discovered again. |
| No row or any current status | `skipped` | A URL matches an exact path in its configured host's `exclude_paths`; it is persisted as rejected at enqueue time. |
| `pending` | `in_progress` | `FrontierDb::claim_next` atomically claims a non-Playwright URL for HTTP work, or the scraper sets a Playwright URL to `in_progress` immediately before opening its browser tab. |
| `in_progress` | `pending` | At crawler startup, `FrontierManager::new` resets all in-progress frontier rows. At each scraper pass, `resetQueue` resets in-progress Playwright rows. `reset_all` also sets every frontier row to pending when the configured full-reset option is enabled. |
| `complete` | `pending` | The dispatcher re-queues stale completed HTTP URLs when `latest_fetch_time` is at or before the configured refetch cutoff. This requeue is explicitly limited to `use_playwright = 0`. |
| `complete` or `failed_terminal` | `pending` | When the article assembler rediscovers a pagination URL, it enqueues it with article priority; completed or terminally failed rows are made pending and their attempt counters are reset for the new refresh cycle. Pending and in-progress rows are left unchanged. |
| `in_progress` | `failed` | A retryable HTTP request, media persistence, or article routing failure occurs and fewer than five attempts have been claimed. The dispatcher re-queues it after its persisted backoff expires. |
| `failed` | `pending` | The dispatcher re-queues a retryable failure after its persisted backoff expires, if fewer than five attempts have been claimed. |
| `in_progress` | `failed_terminal` | An HTTP failure is classified as permanent, or a retryable failure occurs on the fifth attempt. This state is not automatically requeued. |
| `failed_terminal` | `pending` | The article assembler explicitly rediscovers the URL as a pagination page; this starts a new bounded attempt cycle. |
| `in_progress` | `skipped` | After claim, the frontier manager's crawl-policy check returns no crawl decision, so the URL is marked skipped instead of being sent to a worker. |
| Any current status for the article's URLs | `complete` | An HTML article is finalized after its archive JSON is written; `mark_article` applies completion to all URLs associated with that article. A non-HTML response is marked complete after its media file is written. |
| `in_progress` | `complete` | The Playwright loop calls `setStatus(..., 'complete')` after its navigation attempt. It catches a `page.goto` error, logs it, and still closes the tab and marks the URL complete. |
| Any status | `pending` | If the configured full reset is enabled, `FrontierDb::reset_all` resets all frontier rows. |

The article assembler starts a current snapshot only from page 1 (or an
unpaginated root URL). It accepts pagination pages only when they were
discovered during that page-1-rooted refresh; a stale, unlinked page may be
fetched and have its URL status updated, but cannot create or join the current
snapshot. A complete refresh replaces the current page set, so removed pages
remain only in historical snapshots. The previous archive is preserved if any
known page reaches a terminal failure or is excluded by host path policy; the
actor waits for other known pages to resolve, then discards the incomplete
snapshot and releases its router slot.
Retryable failures remain pending in the article assembly until retry success
or retry exhaustion.

The normal HTTP fetch path rejects non-2xx responses with `error_for_status`.
Network errors, save errors, HTTP 5xx, 408, and 429 are retried up to five total
attempts with exponential backoff; other HTTP 4xx and malformed requests are
terminal. HTML `FetchedPage` metadata currently records status code `200`
rather than the actual 2xx code. Playwright work can still be marked complete
after a failed navigation attempt, so do not interpret `complete` as proof
that every fetch or visit succeeded.

`skipped` and `failed_terminal` are not automatically retried. `failed` rows
return to `pending` only after the retry delay, while under the five-attempt
limit. `reset_all` can reset every frontier row to pending, including terminal
failures, and also clears attempt counters and retry delays.

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
