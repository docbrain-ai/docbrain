# How DocBrain Earns Trust

If a tool tells you "Alice owns the payments service" or "documentation improved 12% this month," the only question that matters is: **how do I know that's true and not a confident-sounding guess?**

DocBrain is built around one answer: **it refuses to show you anything it can't stand behind.** Accuracy isn't a promise printed on a dashboard — it's a precondition the system enforces *before* it displays a number. When DocBrain isn't sure, it says so plainly instead of guessing.

This page explains, in plain terms, the five mechanisms that make that true.

---

## 1. It abstains instead of guessing

The most important design choice in DocBrain: **"I don't know" is a first-class answer.**

A confident-but-wrong answer is worse than no answer — it sends someone to the wrong person, or makes a decision on a fabricated number. So DocBrain is built to withhold rather than guess:

- **Ownership** — DocBrain only names an owner when the behavioral evidence is strong enough (the accuracy bar is described in the next section). Otherwise it shows **"No confident owner — not enough signal yet,"** with the reason. It never invents a plausible-looking name to fill the space.
- **Insights and ROI** — When there isn't enough data to be meaningful, you see **"Not enough signal yet — observing,"** not a made-up percentage. Cost-savings estimates are shown as a **range** (for example, "$18–$93"), never a single false-precision figure.
- **Subject matching** — DocBrain matches a question to a known system by exact identity, not fuzzy guessing. If it can't match cleanly, it steps aside and lets normal search answer — it won't risk attaching an answer to the wrong thing.

> **What you'll notice:** a brand-new DocBrain shows mostly "observing" and "no confident owner." That is the system working correctly — it earns the right to make confident claims as real evidence accumulates. A tool that shows confident numbers on day one is the one to be skeptical of.

---

## 2. Confident ownership is gated behind a real accuracy audit

This is the strongest guarantee, and it's worth understanding exactly.

DocBrain will **not display a confident ownership attribution at all** until that capability has been **measured against a human-checked answer key** and passed. The control works like this:

1. **It ships turned off.** Out of the box, confident ownership is hidden. The default is "stay quiet until proven," not "show it and hope."
2. **An operator runs an accuracy audit.** DocBrain compares its own ownership conclusions against a set of human-verified correct answers and measures the **confidently-wrong rate** — how often it would have stated an owner that a human says is wrong.
3. **The bar is strict.** By default, the gate only opens when the audit shows a **0% confidently-wrong rate** across a minimum sample of audited cases (30 by default). Both numbers are configurable, but the shipped posture is "essentially zero tolerance."
4. **A human turns it on, having seen the number.** Opening the gate is a deliberate operator action taken *after* reading the measured accuracy — not an automatic default.

The result: the ownership data you see is data that has been **measured to be ~0% confidently wrong on real labeled examples**, and a person flipped the switch knowing that. Until then, the page honestly shows nothing confident.

The accuracy math underneath is standard statistics for "when is a model allowed to answer" (selective classification / risk-coverage analysis) — not a hand-tuned heuristic.

---

## 3. Every claim shows its evidence

DocBrain doesn't ask you to take its word. Each confident claim carries the evidence behind it, so you can check the reasoning yourself.

- **Ownership** shows *why* a team is named — for example, "resolved 3 incidents, merged 5 pull requests, answered 2 reviews" in that area. You can trace it, and every confident attribution has a **"Report incorrect"** control so a human can push back.
- **Documentation improvement** is the clearest example of this principle. The improvement page states plainly: **"We never claim improvement we can't show."** For each automated fix, DocBrain displays a labeled chain of evidence:

  > published → content actually changed → change confirmed live → a human approved it → measured quality/freshness change

  Each link is shown at its **true strength**. "We published a draft" is labeled as intent, not outcome. A measured improvement is shown only when it can be cleanly measured — otherwise the number is **omitted, never faked or zeroed**.

If DocBrain can't show you the evidence for a claim, it doesn't make the claim.

---

## 4. Humans stay in the loop — and rubber stamps don't count

DocBrain treats human review as real evidence, and it's careful to distinguish genuine review from a formality:

- When a fix is approved, DocBrain records *how* it was approved. A genuine human review counts as strong evidence; an administrative skip-with-bypass is labeled as **weak** — it does not get counted as verification.
- The **"Report incorrect"** controls and ownership-conflict resolution let people correct the system directly, and those corrections feed back in to improve future conclusions.

The system is designed so a human can always overrule it, and so that "someone clicked approve to move on" is never mistaken for "someone checked this."

---

## 5. It's honest about its own confidence, especially when data is thin

DocBrain calibrates its confidence to the evidence it actually has. When data is sparse, it deliberately **lowers** its confidence rather than overstating it — pushing toward "observing / abstain" instead of making bold claims on weak signal. As real activity accumulates, confident claims become available *because they've been earned*, not because a timer ran out.

---

## The honest boundary of the guarantee

Trust is built on being precise about what is and isn't promised. So, plainly:

**What DocBrain guarantees:** it will not *show* you a confident claim it can't justify from evidence and (for ownership) hasn't passed an accuracy audit. The claims it makes are auditable, evidence-backed, and gated.

**What it does not claim:** to be an oracle for social facts. A team can be genuinely ambiguous; a human can declare the wrong owner. That's why DocBrain frames ownership as **"inferred from real activity and accuracy-audited"** — clearly labeled, never presented as absolute ground truth. The honest claim is *"~0% confidently wrong,"* which is a meaningfully different and more defensible promise than *"always right."*

That distinction is the whole point. A tool that promises it's always right is asking for blind trust. DocBrain shows you its evidence, tells you when it doesn't know, and proves its accuracy before it speaks — so the trust is earned, and checkable, every time.

---

## See also

- [Governance](governance.md) — how ownership, stewardship, and SLAs are defined and enforced
- [Autopilot](autopilot.md) — how documentation gaps are detected and fixes are drafted
- [Knowledge Intelligence](knowledge-intelligence.md) — how DocBrain measures coverage, freshness, and quality
- [Review Workflows](reviews.md) — how human approval works
