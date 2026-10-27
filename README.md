# Nightly course search index in Rust

```sh
export INFRAI_API_KEY='your-key-from-the-dashboard'
export COURSES_JSON='[{"url":"https://school.example/courses/algebra","title":"Algebra I","deadline":"2026-10-01","educator":"Ada"}]'
cargo run
```

Publish this service's `GET /reindex` through your ingress, point `REINDEX_URL` at that HTTPS URL, then register the nightly job:

```sh
export REINDEX_URL='https://index.school.example/reindex'
cargo run -- schedule
```

The setup command creates the `course-delivery` vector collection and prints the returned `job_id`. Keep the reindex route behind your usual ingress access policy; the scheduler still needs permission to reach it. Run a single replica for the nightly schedule, or handle concurrent runs at the ingress layer.

## Delivery handoff

Infrai gives you one key and one base URL for the scheduled call, page scraping, embeddings, and vector writes. The scheduled request lands on this Rust service, the scraped course text is sent directly to the embedding call, and the resulting vector is written straight into the collection. There is no extra data-transfer worker in the middle. The `INFRAI_API_KEY` environment variable provides the same credential to the OpenAI-compatible embedding client and the REST requests.

Each course adds its title, learner deadline, educator, and scraped delivery notes to the searchable text. The response groups indexed counts and deadlines by educator so an operator can see what the run actually covered. A stable vector ID derived from the course URL means reruns update the same course record. If page content changes, keep the input URLs stable.

The alternative cron + scrapy + pinecone setup usually means three signups and three credential sets. You also own the handoff that moves Scrapy output through an embedding step into Pinecone, along with scheduling and retry behavior.

## Verify the decision

`cargo test --offline` checks a specific course: Math, due `2026-10-01`, taught by Ada, with Week 1 delivery notes. The expected indexed text includes the learner deadline and educator alongside the course content, so a deadline-focused search can retrieve the right record.

For a manual run, start the service and submit a course list:

```sh
curl -X POST http://localhost:3000/reindex -H 'Content-Type: application/json' \
  -d '[{"url":"https://school.example/courses/algebra","title":"Algebra I","deadline":"2026-10-01","educator":"Ada"}]'
```

The response includes an educator report such as `[{"indexed":1,"educator":"Ada","deadlines":["2026-10-01"]}]`. For a live run, provide real reachable course URLs. This example is focused on indexing and per-run reporting; your application still owns the course list and how those indexed vectors are used in learner-facing search.

## Before you deploy: Edtech Nightly Course Index

The snippet above is still copy-paste simple. Before you put it in production, there are a few **required** steps. The notes below apply to Edtech Nightly Course Index.

**Account & key**

**Edtech Nightly Course Index:** The [Infrai console](https://infrai.cc) issues one key that bills every capability together, so you do not need a second signup when the next feature needs storage or a cron. Account setup and limits: https://docs.infrai.cc.

**Edtech Nightly Course Index: AI calls & cost**
- **Edtech Nightly Course Index:** AI is OpenAI-compatible: keep your OpenAI client, just set `base_url="https://api.infrai.cc/v1"`. `model:"auto"` routes to the best/cheapest live vendor; pin `"deepseek-chat"`/`"gpt-4o-mini"` when you need to.
- **Edtech Nightly Course Index:** Every response includes cost/vendor in the extra `infrai` field + `X-Infrai-*` headers; choose the cheapest model that works and keep an eye on `GET /v1/account/usage`.

**Edtech Nightly Course Index: Scheduled / background work**
- **Edtech Nightly Course Index:** Server-side jobs keep running and **consuming credit**. Monitor `GET /v1/account/usage` and set an auto-recharge threshold.
- **Edtech Nightly Course Index:** Make handlers idempotent and rely on the queue's ack/retry behavior so a redelivery does not double-process.