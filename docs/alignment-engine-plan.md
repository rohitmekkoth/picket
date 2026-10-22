# Alignment engine review and implementation plan

Reviewed: 2026-10-21. Status: the M2 store and M3 mock engine are implemented; M4 evaluation and real-source experiments remain open. This is a focused companion to [design.md](design.md), not a second product design. It replaces the original weighted attribution and authorization proposals. Product scope remains local, read-only Gmail metadata and primary Calendar monitoring, with optional Messages and OpenClaw observations.

## 1. Product decision

Picket can independently observe account state. It usually cannot independently prove who caused it. The engine must answer four separate questions:

1. **Observation:** What did a source actually report, and how complete is our view?
2. **Attribution:** What links that observation to an actor, under which trust assumptions?
3. **Request fit:** Does the visible action satisfy a particular captured request or user-defined policy?
4. **Notification:** What merits the user's attention, with what wording and urgency?

An account observation is not proof of an agent action. An agent claim is not permission. A plausible task match is not attribution. Missing evidence is not evidence that the human acted or that everything was safe.

Retain useful monitoring even without attribution: show account changes, explicit account-policy conflicts, security notices, and collection gaps. Effects-only integrations must advertise this limited capability. Never manufacture per-agent activity totals by assigning unattributed changes to a registered agent.

## 2. Findings in the previous design

| Priority | Problem | Required correction |
| --- | --- | --- |
| Critical | A claim assigns attribution 0.95; an effects-only agent starts at 0.30; lower scores become `user` | Remove these constants and the automatic human verdict. Use evidence states with an explicit unknown actor |
| Critical | The additive matcher can accept a forbidden action: target 0.25 + claim 0.20 + timing 0.15 + unconstrained details 0.10 = 0.70, even with action_allowed = 0 | Constraints are gates. No amount of timing or self-report can cancel a contradiction |
| Critical | Recipient mismatch can similarly be outweighed; overlapping claim time counts as a claim match | Compare the complete observed target set and operation. Preserve contradictions, missing fields, and extra recipients |
| Critical | No captured request becomes `unrequested`, and a missing effect becomes an implied false claim | Use `no_observed_request` and `claim_unverified`. Scope, delay, retention, and collection gaps limit negative evidence |
| High | Sender/subject patterns become factual forwarding, app-access, and purchase events | Store a notice or purchase indicator with its evidence. These do not establish a settings change, app grant, or charge |
| High | Calendar changes are treated as the account owner's actions | Distinguish external organizer updates, invitations, attendee responses, and unknown origin |
| High | Fixed timers assume complete collection; later evidence only revises pending items | Record coverage and revisions. Permit correction after notification without deleting history |
| High | Expected feedback creates implicit trust; contact history helps authorize future actions | Separate expectedness, actor confirmation, and explicit policies. Contacts are context only |
| High | Model output is grammar-valid, therefore treated as semantically reliable | Validate grounding and uncertainty; schema validity does not prove correct interpretation |
| High | The extraction prompt classifies questions as not-a-task | Recognize polite requests such as "Could you draft this?"; distinguish requests from questions about capability or past activity |
| High | Aggregate recall/FPR and 20 seed cases can hide false accusations or an engine that always abstains | Evaluate attribution, request fit, alert quality, abstention, and coverage separately, by integration mode |

## 3. Evidence policy

These are evidence categories, not calibrated probabilities or points to add together. Each item retains its source, resource reference, collection time, trust domain, and limitations. The engine records supporting and conflicting evidence. Two observations copied from the same claim are one origin of evidence, not two independent votes.

| Evidence | What it establishes | Permitted use and limit |
| --- | --- | --- |
| Google resource fetched by Picket | A particular account resource state was observable | Ground the observation. Does not identify the actor or prove delivery/payment |
| Provider audit event with app identity and a unique matching action/resource | Provider-recorded app involvement | Strongest proposed attribution source. Workspace only, future investigation; app identity may still be shared by multiple agents |
| Instrumented tool result with a provider resource ID, operation, account, and matching observed result | A runtime reports a specific tool action consistent with account state | `runtime_linked`, qualified by runtime trust. Stronger than prose but not independent proof against a malicious runtime |
| User explicitly identifies the actor of this action | User-confirmed attribution | Preserve as feedback with provenance, not automatic ground truth for every similar action |
| Agent says it acted | The agent made a claim | `reported_by_agent`; associate only when targets and operation agree. A false or copied claim must not become verified attribution |
| User request, task window, recipients, time | A possible relationship to a task | Candidate selection and request comparison. Does not identify who performed the action |
| Header patterns | Possible composing software/library | Experimental context only. Shared libraries, browser automation, updates, and imitation can defeat separation |
| Mac idle time, usual hours, connected-since date | Local context or registration metadata | Explanation/filtering only. Never establishes agent or human identity; phones and other computers remain invisible |

Suggested actor result:

- `unknown`: no usable actor evidence, including effects-only observations.
- `reported`: one or more agents claim involvement, but there is no stronger linkage.
- `runtime_linked`: supported adapter links a tool result to a specific observed effect under the stated runtime trust assumption.
- `provider_linked`: a tested provider audit integration links the actor/app and specific effect. Unavailable in v1 until separately scoped and verified.
- `user_confirmed`: user explicitly identified the actor; retain the underlying machine assessment too.
- `conflicting`: credible links disagree. Preserve candidates and evidence instead of picking the latest task or highest heuristic score.

Store `ActorRef` separately from evidence strength: human, registered agent, external party, application, or unknown. A shared application ID does not select one agent. Multiple agents claiming the same effect remain unresolved. An untrusted extra claim cannot erase a stronger independent link or lower an existing account-policy alert.

UI examples: "Actor unknown", "Instinct reports sending this", "Linked to an OpenClaw tool call". Reserve an unqualified actor assertion for evidence that actually supports it. Do not display 95% confidence until a held-out calibration study justifies that number for the applicable population.

## 4. What source research establishes

Research below was checked on 2026-10-21. Capability documentation establishes what can be investigated; it does not replace testing the installed versions and account types.

- **Personal Gmail:** The documented message resource exposes IDs, headers, labels, and timestamps, with no sending OAuth application field. `messages.get` supports selected headers under `gmail.metadata`. The API accepts caller-constructed MIME for sending. Thus fingerprints are testable but are not authenticated actor identity. [Message resource](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages), [metadata retrieval](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/get), [sending](https://developers.google.com/workspace/gmail/api/guides/sending).
- **Observation versus send:** Gmail supports importing mail without sending it. Treat a Sent-folder record as an observed record, distinguish baseline/import from live observation, and do not infer delivery to recipients. Do not equate an RFC Message-ID header with Gmail's internal message ID. [Import semantics](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/import).
- **Workspace correction:** The earlier conversation understated its potential. Gmail log documentation includes actor app name, OAuth client ID, event type, and header Message ID. It also documents agent fields with limited product support. OAuth activity logs separately expose app and method. This warrants an experiment, not an assertion that every account, send path, export, or API exposes a unique usable join. [Gmail logs](https://knowledge.workspace.google.com/admin/reports/gmail-log-events), [OAuth activity](https://developers.google.com/workspace/admin/reports/v1/appendix/activity/token). Admin privileges, eligible edition, log latency, missing fields, identifier collisions, and API/export access must be tested. No new admin scope is authorized by this plan.
- **Calendar:** Creator/organizer fields identify calendar participants, not the app responsible for each revision. Organizer updates propagate to attendee copies. Model observed revisions and external changes accordingly. [Event fields](https://developers.google.com/workspace/calendar/api/v3/reference/events), [event propagation](https://developers.google.com/workspace/calendar/api/concepts/inviting-attendees-to-events).
- **Collection gaps:** Gmail history can expire and require full resynchronization; Calendar sync tokens can also become invalid. A new snapshot cannot prove that no transient event existed during the gap. [Gmail synchronization](https://developers.google.com/workspace/gmail/api/guides/sync), [Calendar synchronization](https://developers.google.com/workspace/calendar/api/guides/sync).
- **OpenClaw:** Current documentation describes `after_tool_call`, with optional run ID, tool-call ID, result, error, and duration. Missing error does not prove success; results and coverage depend on the harness. Verify the installed SDK and each supported tool before building the adapter. [Tool observation contract](https://docs.openclaw.ai/plugins/hooks/tool-policy). A socket token authenticates an ingest client, not the truth of its report.

## 5. Experiments before making attribution promises

M2 storage and synthetic engine work can proceed without these results. Strong agent-attribution claims, real-source adapters, and public reliability claims cannot. Preserve the existing mock-first milestone order: specify experiments now, run synthetic simulations in M3/M4, and run account experiments during authorized M5/M6 work. Do not send mail, read real accounts, add scopes, or buy services as part of this design review.

### E1. Header separability

Use synthetic content and controlled, independently labeled sends from Gmail web/mobile, Apple Mail/Outlook where available, an ordinary API sender, OpenClaw, and Instinct/Muse only when access actually exists. Include draft-then-send, replies, plain/multipart mail, attachments, scheduled sends, browser-driven agents, two tools sharing a library, missing headers, duplicates, and a changed client version. Never treat pre-agent mail as human by definition.

Start with `Message-ID`, top-level `Content-Type`, `MIME-Version`, and `X-Mailer` if present. These are candidate headers; nested MIME/body inspection remains out of scope. Record bounded structural features, not unique boundary values or full identifying headers, for fingerprint learning. Preserve only the separately necessary message identity for joins. Raw research outputs stay outside Git; committed fixtures must be synthesized. Missing or new patterns mean unknown, not agent.

Use an operator's send ledger as labels. Split by session/day, account and client version, not random messages from the same template. Report false attribution, abstention, and coverage per client and mode. A small day-one sample is a feasibility probe only. If patterns overlap, leave fingerprints diagnostic-only. Do not enable a production classifier just because a few examples separate.

### E2. OpenClaw action linkage

Pin an actual supported version and tool. Verify the hook runs for that tool, successful results contain the provider ID, and Picket's independent read matches account, action, targets, and resulting state. For Calendar updates, event ID alone is insufficient: require the relevant revision or an unambiguous transition. Test retries, partial success, async tools, direct shell/browser bypass, result truncation, absent IDs, forged claims, dropped observations, and runtime restart.

Add adapter instance ID, monotonic sequence, and loss counters to bounded plugin reports. Gaps and a stale heartbeat degrade coverage. A healthy heartbeat still does not prove every operation was observed. Do not forward raw arguments, tool output, tokens, bodies, or arbitrary URLs. Unknown tool outcomes stay unknown.

### E3. Workspace attribution, future optional work

For an eligible test tenant, verify that an actual Send event includes a useful app identity and a resource join, and determine whether the documented fields are available through the intended API or export. Test sends via web, API, delegated/shared clients, scheduled sends and duplicate RFC Message-IDs. Record missingness, delay, cost, edition and privilege requirements. App identity is not necessarily the human who initiated a workflow. Keep this out of v1 permissions and implementation until separately authorized.

### E4. Request capture and Calendar behavior

For each claimed supported integration, establish which user requests and edits are observable, whether identities/conversation direction are reliable, and what happens across devices, delayed sync, deleted messages, and unsupported channels. Verify Calendar invitations, organizer changes, recurring exceptions, resync and repeated updates. Unsupported sources must be visibly labeled. Instinct/Muse request capture remains a product-specific experiment, not an assumed capability.

## 6. Deterministic decision pipeline

Use ordinary typed Rust functions and explicit state, not a generic rules interpreter or a probabilistic attribution framework in v1:

```text
typed observations + coverage + versioned extraction + explicit feedback
    -> account-policy checks
    -> actor evidence resolution
    -> task candidate selection and constraint comparison
    -> claim consistency checks
    -> findings with evidence and uncertainty
    -> severity/routing -> persisted notification outbox
```

### Request representation and comparison

Represent action, target set, time, quantity, currency/amount, prohibitions, conditions, and approval requirements independently. Each field is `known`, `unknown`, or `ambiguous`, with a source reference and extraction provenance. `null` is never a wildcard permission. Distinguish draft from send, create from update, and singular permission from unlimited repetition. Never infer an email address from a business name or authorize anyone merely because their address is a known contact.

Keep task revisions and valid-time intervals. A later follow-up changes future scope; it does not retroactively authorize an earlier action. Explicit user feedback can mark an earlier action expected without rewriting the original request assessment. Agent messages cannot widen a user's permissions. If follow-up identity or cancellation scope is ambiguous, retain alternatives and abstain.

For each relevant task, evaluate independently:

1. Establish account, operation, task relation, temporal applicability, and evidence availability. Missing fields are explicit.
2. Compare all observed recipients/attendees with the permitted set, including available CC/BCC and extra targets. Missing target visibility prevents a complete match. Exact-match address handling must not conflate aliases without explicit mappings.
3. Evaluate prohibitions and required conditions first. A known violation produces `conflicts_with_request`; later positive evidence cannot cancel it.
4. Return `consistent_with_visible_request` only when the relevant visible fields agree and no known condition fails. Attach extraction uncertainty and a list of unassessed dimensions. This is not a guarantee of complete authorization or semantic safety.
5. Use `insufficient_evidence`, `ambiguous_task`, or `no_observed_request` when appropriate. A merely nearby unrelated human email must not be compared as if the agent performed it.

The result is comparative: "This recipient differs from the captured request; actor unknown" is valid. "Instinct sent an unauthorized email" requires substantially stronger evidence for both actor and authorization. Never hide a clear contradiction in one relevant task by choosing a more permissive unrelated task. Do not count one observed effect as completion of multiple independent requests, or let one permitted send excuse unlimited duplicates. Track explicit action slots where known; unclear cardinality remains unknown.

Task association must also have a reason: matching provider thread/resource, a linked tool invocation, an explicit user selection, or a specific claim with compatible targets. A broad time window alone supports only a possible relationship. If only that relationship exists and the actor is unknown, retain the conditional comparison in the timeline; do not push a personal-task violation alert. A confirmed account-wide policy can still apply independently of any task or actor.

No fuzzy additive score decides permission. Candidate ranking may later help performance, but all hard checks and ambiguity handling remain mandatory. If a candidate search hits its configured bound, report incomplete assessment instead of silently excluding alternatives.

### AI boundary and privacy

The local model proposes structured request/claim fields. Rust validates schema, field sizes, enums, source spans when available, time interpretation, and contradictions. Negation, conditional approval, quoted text, sarcasm, multiple recipients, and follow-up resolution need specific tests. Model self-confidence is not evidence. Formatting constraints and delimiters do not eliminate prompt injection or hallucination.

The former AI judge becomes an optional advisory explanation step, disabled for v1 verdict changes. It cannot establish an actor, add permission, erase a contradiction, close a finding, or downgrade severity. Keyword fallback can surface candidate tasks but cannot turn a keyword into granted permission. Model timeout or failure yields an explicit degraded assessment.

Resolve dates against the request's source time and known timezone, not today's processing time. Preserve uncertainty across travel and DST. Store validated structured extraction, source IDs, bounded snippets allowed by settings, model/prompt version, and a record that raw text was discarded. Raw Messages text is transient and must not enter logs, exports, crash artifacts, or the durable notification queue.

This creates an unavoidable tradeoff: replay can reproduce decisions from stored extraction, but cannot re-extract discarded original text. A 120-character snippet cannot always show the full original instruction. Do not promise full-text review or exact model replay under this privacy policy. Truncated/unavailable supporting text must be visible to the user. Changes to retention or raw-text storage require a separate product decision.

### Claims and negative evidence

- Compare what the agent reports with observed effects separately from request fit.
- Link claims using operation, targets and resource identity where available. Time alone is insufficient.
- Use "Claim not verified in the observed sources" instead of "Agent lied". Secondary calendars, missing message bodies, source gaps and delayed effects are competing explanations.
- Use "No matching claim observed" only with the relevant channel's coverage and a wait interval. Absence of a claim alone is low priority unless the user explicitly requires reporting.
- A captured user request can itself be inferred or incomplete. Preserve extraction uncertainty even though final policy evaluation is deterministic.

## 7. Lifecycle, storage and replay contract

M2 should establish the minimal durable envelope needed below; engine-specific tables can be added by M3 migrations. Do not add unused placeholder modules or unbounded payload fields.

| Record | Required information |
| --- | --- |
| Observation | Schema version, stable account/source IDs, provider resource and revision identity, observed fields with missingness, source-time interval/quality, ingest time, baseline/live/catch-up origin |
| Evidence | Referenced observations/claims, asserted actor, operation and result linkage, trust domain, adapter/version, supporting or conflicting status |
| Coverage | Source/account/capability, cursor/checkpoint, last successful collection, known gaps, bounded scope, overflow/loss and resync state; never imply global completeness |
| Extraction | Source message ID and revision, structured fields with uncertainty, model/prompt/schema versions, allowed snippets and retention status |
| User policy/feedback | Version and effective time, explicit expectedness separate from actor label, scope and undo history |
| Assessment | Observation + task revision, actor result, request-fit result, coverage refs, reason codes, policy/engine versions and decision revision |
| Finding/outbox | Stable finding key, incident grouping, revision, routing state, notification idempotency key and delivery attempt state |

Use account-scoped provider IDs; RFC Message-ID is a fallible secondary join key. Calendar identity needs calendar ID, event/instance ID and revision. Neither `updated` alone nor the base event ID represents every change. Keep the original four M2 decisions about deduplication, contacts, nullable finding keys, and retention/feedback; add this evidence contract before freezing schema.

Process each durable input in sequence with assessment updates and the engine cursor in one SQLite transaction. Append bounded extraction results, coverage changes, policy revisions and feedback as replay inputs. Rebuild with the stored versions; replay under newer rules is a distinct assessment run. Retention creates a bounded evidence horizon and checkpoints; do not infer missing requests/claims outside it. Feedback uses stable finding/observation identities so replay does not apply it to unrelated incidents.

Use an injected clock. Wait five minutes from the later of reliable source event time and first local observation for ordinary mismatches, with up to five additional minutes for relevant queued extraction or catch-up. A suspicious future timestamp uses local observation time and records clock uncertainty; it cannot postpone assessment indefinitely. Immediate account-policy alerts need not wait. Timer expiry changes pending to assessed or incomplete; it never certifies source completeness. Late evidence can revise any finding, including a notified one. Persist the revision and notify again only for a material correction or escalation. A delayed historical batch goes to a catch-up summary rather than a burst of old alerts.

Persist outbox work with the finding transaction. Retry delivery with stable identifiers. Exactly-once display is not guaranteed across a crash between OS delivery and acknowledgement; the incident store is authoritative, and retries should minimize duplicates. Pausing, sleep, clock skew, expired cursors, rate limiting and disk failures must visibly degrade coverage.

## 8. Severity, feedback and UI

Severity expresses consequence; evidence status expresses what is known. Neither substitutes for the other. Avoid a single heuristic score that combines both.

| Condition | Default handling |
| --- | --- |
| Clear observed breach of an explicit user-confirmed account policy | Immediate high if the policy designates high consequence; actor may remain unknown |
| Strongly linked effect conflicts with a user-confirmed restriction or marked revocation boundary | High when consequence warrants; title retains runtime/provider qualification |
| Conflict derived from an unconfirmed model interpretation, or weaker actor evidence | Medium review with precise uncertainty; no unqualified accusation |
| Possible personal-task conflict based only on timing/context, with unknown actor | Timeline/digest with conditional wording; no push alert. Requires stronger association or an explicit account policy for escalation |
| Security-related email detected by metadata patterns | Prompt medium review as a security notice, not a proven settings change; never title it as agent-caused |
| Unknown actor, no clear conflict, or claim not yet verified | Timeline/digest; do not turn every unknown action into an alert |
| New recipient, unusual hours, or unusual volume alone | Context/digest by default; not proof of wrongdoing |
| Collector failure, overflow, or missing request channel | Distinct coverage warning, aggregated to avoid repeated noise |

"Revoked" in Picket means the user recorded revocation; Picket cannot revoke or verify another app's access with current scopes. Action initiation and completion may straddle that time. If timing is uncertain, say so; do not imply Google failed to enforce revocation.

Keep expectedness and actor feedback separate. "This was expected" resolves that finding only; it does not teach a fingerprint, whitelist a contact, or change thresholds. An explicit optional "Who performed this?" answer is separately recorded and reversible. Disable automatic online learning in v1 to avoid feedback loops and poisoning. Future learned models require independent labels, offline evaluation, versioning and rollback.

Incident views show observed facts, the captured request summary/snippet, the agent's claim, attribution evidence, contradictions, source coverage and what could not be checked. Render bounded reason codes into fixed wording; no model-written accusation as a title. A calm tray means "No current alerts in observed sources", not "All actions authorized". Count unknown and externally initiated changes separately from agents. Group related findings without dropping individual effects or hiding higher severity.

## 9. Coding increments and dependencies

Current code is the data-free M1 shell. `crates/core/src/lib.rs` exposes runtime metadata only. Build domain behavior in the portable core, reuse it from the CLI and app, and keep macOS and network access in adapters. Names below describe responsibilities, not mandatory empty files to create now.

| Increment | Deliverable | Acceptance before the next increment |
| --- | --- | --- |
| P0: contract and fixtures | Finalize typed observation, evidence, coverage, request and verdict schemas; author adversarial scenario labels first | Review the decision examples and uncertainty behavior; no unsupported actor/probability assertions in the contract |
| M2: durable evidence foundation | `events`, `canon`, `store`; migrations, writer, retention/checkpoints, `picket verify`, replay input versions | Existing M2 tamper and crash tests plus account-scoped identity, revision, coverage-gap and feedback survival cases |
| M3a: replay and account rules | `engine/rules`, `engine/coverage`, injected clock, minimal CLI replay and `MockModel` | Same ordered input yields identical findings; account rules work with AI/agent channels disabled |
| M3b: actor evidence | `engine/attribution`; explicit states, trust domains, conflict handling and exact linkage validation | Human phone activity and effects-only cases remain unknown; copied/fabricated claims never become independent evidence |
| M3c: task constraints | `engine/tasks`, `engine/matcher`; tri-state fields, revisions, prohibitions, target sets and action slots | Forbidden action/extra recipient cannot be outscored; late follow-up cannot retroactively authorize; ambiguity preserved |
| M3d: claims and incidents | `engine/claims`, `engine/incidents`; grace periods, late revisions, stable keys, outbox | Replay, restart and duplicate events do not duplicate findings; incomplete coverage never produces a conclusive absence claim |
| M4: evaluation gate | `eval/scenarios`, CLI report, versioned baseline and CI metrics | Held-out partitions and per-mode safety/utility gates pass; zero-result/all-unknown engine fails utility gates |
| M5: Google observation | Existing scoped OAuth and collectors; distinguish baseline, live and resync; synthetic wire fixtures then controlled account checks | Events match observed source semantics; resync/partial visibility is shown; execute E1 only within approved account work |
| M6: supported request/runtime adapters | Messages decoder and pinned OpenClaw tool adapter; E2/E4 experiments | Prove each adapter's actual capability; unsupported/missing IDs or dropped events visibly degrade, not invent certainty |
| M7: local extraction | Bounded worker, validated extraction, adversarial model evaluation and degraded fallback | Model errors do not grant permission or change actor evidence; no raw-text leakage; real-model results reported separately |
| M8: user flow | Evidence views, separate feedback, uncertainty wording, notification revisions and coverage status | End-to-end human-versus-agent scenarios display the right evidence; browser and packaged macOS checks pass |
| Release decision | Mode-specific test report, documented scope and limitations | Enable only modes that meet gates; do not market general agent attribution when only account monitoring is validated |

Keep each increment a coherent branch/commit series with focused tests and review of staged data. Run `make check` and `make test` for core increments; add the real `make eval` target when the harness exists. Run `make e2e` for UI changes, `make smoke-macos` for native lifecycle/notification changes, and `make audit` for dependencies and before merging the complete branch. Use synthetic fixtures and generators. Do not add placeholder CI baselines or weaken existing gates.

Useful acceptance examples before coding:

| Inputs visible to Picket | Expected result |
| --- | --- |
| Human sends from phone while Mac idle; an agent is registered | Actor unknown; no agent count or personal-task accusation |
| "Draft only" request; supported runtime reports send with a matching observed resource | Runtime-linked actor; explicit send/draft conflict; no claim/timing score can make it consistent |
| Same send with only an agent's prose claim | Reported actor; qualified claim/request conflict for review, never independently verified actor |
| Calendar invitation changes after its organizer edits it | External-origin context; do not assign the change to the user's agent |
| No message-channel access; unexplained Sent record | No observed request, actor unknown; no invented unauthorized verdict |
| User confirmed an account policy forbidding a particular recipient; observed target violates it | Account-policy finding even with actor unknown; policy scope is explicit |
| Two conflicting agent reports refer to one resource | Preserve competing reports; no arbitrary winner or double counting |
| A late request or resync changes a notified finding | New assessment revision and material correction, preserving the original evidence/history |

Keep delivery bounded: cap candidates, model input/output, pending work, rule evaluation, and retries; overflow is a recorded coverage/assessment failure. Coalescing messages must not introduce the former ten-minute extraction delay into a six-minute finding target. Measure the whole pipeline on representative hardware before choosing final limits.

## 10. Evaluation and release gates

Ground truth must describe the real actor, initiation/completion times where known, user intent, expected effect and visibility. Provide the engine only the evidence that would actually be observable. Include paired scenarios with identical visible evidence but different true actors; both must yield unknown. Independent operator labels and held-out data are essential. Generated variants of one template are not independent validation samples.

Minimum scenario families, with both benign and adverse variants:

- Human sends from phone while Mac idle; user uses several clients; another automation runs; agent drives the human's browser.
- Two agents share a mailbox/client library, have overlapping tasks, or claim the same action; new unknown agent; external Calendar organizer updates.
- Wrong action, draft-versus-send, extra CC/BCC/attendee, wrong recipient, homograph/unmapped alias, wrong date/time/DST, wrong currency/amount, too many repeated actions, approval conditions and negation.
- User edits/cancels a task; broad standing instruction; missing/deleted/out-of-channel request; long-running task; post-revocation completion of an earlier action.
- Claim before/after effect, no claim, fabricated provider ID, duplicate/reused RFC Message-ID, partial tool failure, success without an observable resource, missing hook output, replayed ingest token client, copied claims.
- Sleep, clock skew, late Messages sync, expired Gmail/Calendar cursor, out-of-order pages, deleted/transient resources, baseline imports, recurring Calendar changes, disabled collector, queue overflow, retention and crash recovery.
- Model injection, malformed output, valid JSON with wrong meaning, polite requests phrased as questions, truncated source, missing timezone, unsupported language, timeout and AI disabled.
- Feedback reversal, poisoned expectedness labels, new mail client/version, misleading sender/subject security notices, auto-replies and legitimate notices.

Metrics must include numerators, denominators, unknown counts, and uncertainty intervals:

| Measure | Definition / initial gate |
| --- | --- |
| Safety invariants | Zero violations of forbidden-action, actor-evidence, missingness, trust-boundary, replay and privacy invariants in unit/property/integration tests |
| Named-agent precision | Correct named-agent assertions / all named-agent assertions, separately by evidence class and integration. Proposed production gate: one-sided 95% lower bound at least 99% |
| False blame on human actions | Human actions assigned to an agent / labeled human actions. Proposed gate: one-sided 95% upper bound at most 1%, including phone and other-client slices |
| Evidence-qualified attribution | Report accuracy of `runtime_linked` separately from provider/user-confirmed cases, and explicitly state the honest-runtime assumption; adversarial cases must not be advertised as certified |
| Attribution coverage | Asserted/linked actors / all observed effects, plus unknown and conflicting rates. For the declared supported instrumented path, proposed useful-linkage target at least 80%; no minimum for effects-only modes, which cannot claim attribution |
| Request-fit safety and utility | False-consistent rate on explicitly conflicting tasks must be zero in the deterministic suite; proposed held-out conflict recall at least 90% and alert FPR at most 5% within declared observable scope |
| Alert precision and burden | Report precision by severity, false alerts per 100 benign effects, and alerts/day under a realistic base rate. A balanced synthetic dataset alone cannot establish practical precision |
| Extraction | Per-field error rates, particularly omitted prohibitions/targets and invented permission; do not accept aggregate 90% accuracy as a substitute for critical-field safety |
| Coverage | Gaps/unsupported fields surfaced in every applicable test; report missed observations separately from engine misses |
| Latency | Effect-to-ingest, ingest-to-assessment and assessment-to-notification separately; p50/p95. Retain awake targets of 90 seconds median for immediate rules and 6 minutes for ordinary findings; report extra waits, sleep and backlog separately |

These are proposed acceptance thresholds, not measured results. A 20-case seed set is only an M3 smoke gate; build at least the original 200-scenario M4 suite with held-out families, then collect controlled validation data before release. A 99% one-sided precision lower bound with zero errors requires roughly 299 independent labeled assertions; repeat variants and correlated messages do not provide that assurance. Report small strata as insufficient evidence rather than pooling them into a reassuring average. Do not tune on the test set. Record the chosen interval method and sampling assumptions in the harness.

CI should enforce invariants, per-class regressions and minimum applicable-case counts, while refusing an all-abstain engine that fails supported-case recall/coverage. Keep held-out mock and real-model reports distinct. Model identity, quantization, prompt, parser, adapter, rules and schema versions belong in every report. A model/adapter change invalidates only the relevant established results and must rerun those gates. Use `not_measured` for unrun experiments, never fabricated metrics.

## 11. Limits that remain unsolved

| Limit | Honest behavior / what would change it |
| --- | --- |
| Closed agent and human produce identical metadata through the same account/client | No algorithm can distinguish them from these observations. Keep actor unknown; stronger provider provenance or separately instrumented identities would be required |
| Agent can control its plugin, browser or runtime | Local claims and fingerprints can be forged or omitted. Independent observation verifies an effect, not the claimed actor; trusted execution/provider provenance would change the boundary |
| Account or device is compromised | Local records, labels and app state can be changed. This remains outside the local same-user threat model |
| Instructions exist outside captured channels, or are vague | Cannot establish complete authorization. Compare only captured scope and say what is missing |
| Email content is wrong, private information is disclosed in a body, or a harmful read causes no visible change | Metadata-only read access cannot assess this. More content/access telemetry would change privacy/scope; not part of this plan |
| Mail receipt/notice is missing, misleading, spoofed, or merely a confirmation request | Cannot establish payment, final forwarding configuration, or OAuth state from the pattern alone |
| Polling misses transient changes, source history expires, or the Mac is asleep | Resync recovers available state, not necessarily all intermediate activity. Show a gap; no promise of complete forensic history |
| Text was discarded by privacy policy | Cannot rerun extraction or display full original evidence later; retain structured assessment and disclose the limitation |
| Agent writes outside Gmail/primary Calendar, or no usable request/log integration exists | Unsupported visibility. Do not advertise comprehensive monitoring of Instinct, Muse, or every OpenClaw tool |

The next implementation action is M4: evaluate the mock engine with held-out scenario families, denominators, utility gates and separate mode reports. The next attribution research action is the controlled E1/E2 capability probe during M5/M6. Failed probes narrow supported claims; they must not be compensated for by increasing heuristic confidence.
