# M7 local extraction plan

Status: increments 1-3 implemented on `feature/m7-extraction` (PR #16); Qwen3.5 4B selected, see [decisions](decisions.md#m7-extraction-model-and-validation-2026-10-27). The owner prioritized Instinct, and Meta Muse if possible; the OpenClaw increment of M6 is deferred. This plan implements M7 in [design.md](design.md) under the AI boundary in [alignment-engine-plan.md](alignment-engine-plan.md).

## Outcome

Picket turns the user's messages to an iMessage agent into structured requests, and the agent's replies into structured claims, on the Mac, with a local model. The engine then compares requests, claims and observed Gmail and Calendar effects. Text is read from Messages only while extracting and is never stored; the stored extraction carries its source message identity, model and prompt versions, and grounding.

## Verified facts that shape the design (2026-10-27)

- **Instinct** is reached through iMessage or WhatsApp. With Google connected it can draft, reply and send through the user's Gmail and manage Calendar. Since September 2026 it also sends from its own address on `mail.instinct.com`, for example to businesses. Those messages never appear in the user's Gmail.
- **Meta Muse** is reached through the Muse app, muse.ai, a Mac app or WhatsApp, runs actions in Meta's cloud and connects to Gmail and Google Calendar. Picket cannot read its requests or reports.
- **Models:** Qwen3.5 (Apache-2.0; 4B and 9B) and Gemma 4 E4B (Apache-2.0) are current small candidates. Vendor benchmarks are not evidence for this task; the choice is made on Picket's own extraction evaluation.
- **Runtime:** `llama-cpp-2` 0.1.160 (MIT or Apache-2.0) is maintained, supports Metal and grammar-constrained output.

## Roadblocks and decisions

1. **Claims outside the observed mailbox.** A claim about an email Instinct sent from its own address must not become "not observed in a healthy source". Claims gain a sending channel: the user's account, the agent's own account, or unknown. Only claims about the user's account are checked against Gmail; others are recorded as outside observed scope.
2. **Other request channels.** Requests sent through WhatsApp or the Instinct dashboard are invisible. "No observed request" is worded and scoped as "none in your iMessage conversation with Instinct", and request-capture coverage names that channel.
3. **Meta Muse is effects-only.** Registration supports effects-only agents. Muse effects stay "actor unknown"; account policies still apply. Whether the Muse Mac app keeps readable local history is an open question, not an assumption; it is not pursued without a documented, stable format.
4. **People are named, not addressed.** Requests say "email Sam". Picket never infers an address from a name. Extraction keeps a target as either an address or a name. A named target matches an effect recipient only through that recipient's display name in the effect's own headers (Gmail `To`/`Cc`, Calendar attendee `displayName`), and the match is reported as a name match. An effect recipient that matches no requested name or address is an extra target.
5. **Hallucination.** Every extracted target, and every date or time, must be grounded in the source text: the string, or its digits, must appear in the message. Ungrounded fields become unknown. Model confidence is not evidence.
6. **Prompt injection.** Direction separates trust: the user's messages can only produce requests and approvals; the agent's messages can only produce claims. An agent message cannot grant permission, approve a task or edit a request, whatever its text says.
7. **Tasks and follow-ups.** The model proposes whether a user message starts a task, revises an open task (including approval or cancellation), or is not a task. Rust assigns stable task ids from the first message's identity and keeps revisions; an unresolved follow-up keeps alternatives and the engine abstains.
8. **Time.** The model returns local date-times relative to the message's own timestamp, which is in the prompt; Rust resolves them with the Mac's time zone at that instant and rejects implausible values. Relative words are resolved against the message time, never the processing time.
9. **Edits.** Each message revision is extracted; a later revision supersedes the earlier one for future assessment without rewriting it. Re-extraction later can see only the current text.
10. **Without the model.** Extraction is unavailable, request capture never becomes healthy, and findings say the request channel was not captured. Keyword fallback is not added: it could suggest tasks but could not ground fields, and the design forbids it from granting permission.
11. **Download and integrity.** The model is downloaded only after the user asks, from Hugging Face over an allowlisted host set, pinned to a repository commit and SHA-256, resumable, into Application Support. Redirect hosts are verified during implementation.
12. **Resources.** One worker, concurrency one, bounded queue, per-call timeout and token limits, model loaded on demand and unloaded after idle. Thinking mode is disabled; output is grammar-constrained JSON, then validated in Rust.
13. **Evaluation honesty.** A committed synthetic set of labeled messages, including negations, conditions, approvals, cancellations, quoted text, multiple recipients, relative dates and injection attempts, measures field accuracy and, critically, ungranted-permission errors. Real accuracy is measured separately on owner-approved scripted test conversations. CI uses a scripted model; the real model is evaluated locally.
14. **Live validation needs overlap.** Instinct must act in the Google account Picket monitors. Scripted tests use requests the owner chooses to share.

## Increments

1. **Contract and pipeline on a scripted model (core).** Extraction schema and Rust validation (grounding, direction separation, time resolution, task revisions); claim channels and outside-scope status; named targets with display-name matching from Gmail and Calendar headers; request-capture coverage written when extraction completes; effects-only agent registration. Engine, incremental and scenario tests updated; new evaluation families.
2. **Labeled extraction set and harness.** Synthetic labeled messages and `picket eval --extraction` with field-level and safety metrics; scripted-model run in CI.
3. **Local model runtime (app).** Verified download, llama.cpp worker with Metal and a JSON-schema grammar, prompts versioned in `crates/core/prompts`, transient text reads from Messages by message identity.
4. **Model choice and live validation.** Run the harness on the candidates on this Mac, choose by safety then accuracy then resources, record results; then scripted Instinct conversations end to end.
