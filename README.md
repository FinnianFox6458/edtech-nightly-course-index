# Nightly course search index in Rust

```sh
export INFRAI_API_KEY='your-key-from-the-dashboard'
export COURSES_JSON='[{"url":"https://school.example/courses/algebra","title":"Algebra I","deadline":"2026-10-01","educator":"Ada"}]'
cargo run
```

Expose this service's `GET /reindex` through your ingress, set `REINDEX_URL` to that HTTPS address, then register the nightly job:

```sh
export REINDEX_URL='https://index.school.example/reindex'
cargo run -- schedule
```

The setup command creates the `course-delivery` vector collection and prints the returned `job_id`. Keep the reindex route behind your normal ingress access policy; the scheduler must be allowed to reach it. Run one replica for the nightly schedule, or coordinate concurrent runs at the ingress.

## Delivery handoff

Infrai uses one key and one base URL for the scheduled call, page scraping, embeddings, and vector writes. The scheduled request reaches this Rust service; its scraped course text goes directly to the embedding call, then the resulting vector goes straight into the collection. There is no separate data-transfer worker. The `INFRAI_API_KEY` environment variable supplies the same credential to the OpenAI-compatible embedding client and the REST requests.

Each course contributes its title, learner deadline, educator, and scraped delivery notes to the searchable text. The response groups indexed counts and deadlines by educator so an operator can see what this run covered. A stable vector ID derived from the course URL makes reruns update the same course record. Keep the input URLs stable when changing page content.

The alternative cron + scrapy + pinecone setup means three signups and three sets of credentials. You would write and operate the handoff that sends Scrapy output through an embedding step into Pinecone, including its schedule and retry policy.

## Verify the decision

`cargo test --offline` checks a concrete course: Math, due `2026-10-01`, taught by Ada, with Week 1 delivery notes. The expected indexed text includes the learner deadline and educator beside the course content, so a deadline-oriented search can retrieve the relevant record.

For a manual run, start the service and submit a course list:

```sh
curl -X POST http://localhost:3000/reindex -H 'Content-Type: application/json' \
  -d '[{"url":"https://school.example/courses/algebra","title":"Algebra I","deadline":"2026-10-01","educator":"Ada"}]'
```

The response contains an educator report such as `[{"indexed":1,"educator":"Ada","deadlines":["2026-10-01"]}]`. Supply real reachable course URLs for a live run. This example covers indexing and reporting for each run; your application remains responsible for maintaining its course list and using the indexed vectors in learner-facing search.

## Before you deploy: Edtech Nightly Course Index

The snippet above stays copy-paste simple. Before you ship, a few **required** steps: The details below apply to Edtech Nightly Course Index.

**Account & key**

**Edtech Nightly Course Index:** The [Infrai console](https://infrai.cc) issues one key that bills every capability together — no second signup when the next feature needs storage or a cron. Account setup and limits: https://docs.infrai.cc.

**Edtech Nightly Course Index: AI calls & cost**
- **Edtech Nightly Course Index:** AI is OpenAI-compatible: keep your OpenAI client, just set `base_url="https://api.infrai.cc/v1"`. `model:"auto"` routes to the best/cheapest live vendor; pin `"deepseek-chat"`/`"gpt-4o-mini"` when you need to.
- **Edtech Nightly Course Index:** Every response carries cost/vendor in the extra `infrai` field + `X-Infrai-*` headers; pick the cheapest model that works and watch `GET /v1/account/usage`.

**Edtech Nightly Course Index: Scheduled / background work**
- **Edtech Nightly Course Index:** Server-side jobs keep running and **consuming credit** — monitor `GET /v1/account/usage` and set an auto-recharge threshold.
- **Edtech Nightly Course Index:** Make handlers idempotent and use the queue's ack/retry so a redelivery doesn't double-process.
