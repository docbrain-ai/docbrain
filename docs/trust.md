# How DocBrain Earns Trust

If a tool tells you "Alice owns the payments service" or "documentation improved 12% this month," there's really only one question worth asking: how do you know that's true, and not a confident-sounding guess?

Here's the short answer. DocBrain is built to refuse to show you anything it can't stand behind. Accuracy isn't a promise on a dashboard. It's a precondition the system checks before it shows you a number. When DocBrain isn't sure, it says so instead of guessing.

This page walks through the five things that make that real.

---

## 1. It abstains instead of guessing

The most important decision in DocBrain: "I don't know" is a real answer.

A confident wrong answer is worse than no answer. It sends someone to the wrong person, or it bakes a fabricated number into a decision. So DocBrain holds back rather than guess:

- **Ownership.** DocBrain only names an owner when the evidence is strong enough (more on that bar in the next section). If it isn't, you get "No confident owner, not enough signal yet," along with the reason. It won't invent a plausible name to fill the gap.
- **Insights and ROI.** When there isn't enough data to mean anything, you see "Not enough signal yet, observing." Not a made-up percentage. Cost savings show as a range, like "$18 to $93," never a single false-precise figure.
- **Subject matching.** DocBrain matches your question to a known system by exact identity, not fuzzy guessing. If it can't match cleanly, it steps aside and lets normal search handle it rather than risk attaching the answer to the wrong thing.

One thing you'll notice: a brand-new DocBrain shows mostly "observing" and "no confident owner." That's the system working, not failing. It earns the right to make confident claims as real evidence builds up. A tool that shows confident numbers on day one is the one to worry about.

---

## 2. Confident ownership is gated behind a real accuracy audit

This is the strongest guarantee, and it's worth understanding exactly how it works.

DocBrain won't show a confident ownership attribution at all until that capability has been measured against a human-checked answer key and passed. Here's the flow:

1. **It ships turned off.** Out of the box, confident ownership is hidden. The default is "stay quiet until proven," not "show it and hope."
2. **Someone runs an accuracy audit.** DocBrain compares its own ownership conclusions against a set of human-verified correct answers, and measures the confidently-wrong rate: how often it would have named an owner that a human says is wrong.
3. **The bar is strict.** By default the gate only opens at a 0% confidently-wrong rate, across a minimum sample of audited cases (30 by default). Both numbers are configurable, but the shipped posture is basically zero tolerance.
4. **A person turns it on, having seen the number.** Opening the gate is a deliberate action someone takes after reading the measured accuracy. It's not an automatic default.

So the ownership data you see has been measured to be roughly 0% confidently wrong on real labeled examples, and a person flipped the switch knowing that number. Until that happens, the page honestly shows nothing confident.

The math underneath is the standard statistics for "when is a model allowed to answer at all" (selective classification, risk-coverage analysis). It isn't a hand-tuned heuristic.

---

## 3. Every claim shows its evidence

DocBrain doesn't ask you to take its word. Each confident claim carries the evidence behind it, so you can check the reasoning yourself.

- **Ownership** shows why a team is named. Something like "resolved 3 incidents, merged 5 pull requests, answered 2 reviews" in that area. You can trace it, and every confident attribution has a "Report incorrect" button so a person can push back.
- **Documentation improvement** is the clearest example. The improvement page says it plainly: we never claim improvement we can't show. For each automated fix, DocBrain shows a labeled chain of evidence:

  > published, then content actually changed, then the change confirmed live, then a human approved it, then a measured quality or freshness change.

  Each link shows its true strength. "We published a draft" is labeled as intent, not outcome. A measured improvement only shows up when it can actually be measured. If it can't, the number is left out, not faked or zeroed.

If DocBrain can't show you the evidence for a claim, it doesn't make the claim.

---

## 4. Humans stay in the loop, and rubber stamps don't count

DocBrain treats human review as real evidence, and it's careful to tell the difference between an actual review and a formality.

When a fix is approved, DocBrain records how it was approved. A genuine human review counts as strong evidence. An administrative skip-with-bypass gets labeled weak, and doesn't count as verification.

On top of that, the "Report incorrect" controls and the ownership-conflict resolution let people correct the system directly, and those corrections feed back in to improve later conclusions.

The point is simple: a person can always overrule the system, and "someone clicked approve to move on" is never mistaken for "someone actually checked this."

---

## 5. It's honest about its own confidence, especially when data is thin

DocBrain matches its confidence to the evidence it actually has. When data is sparse, it lowers its confidence on purpose instead of overstating it, and leans toward "observing" or "abstain" rather than making bold claims on weak signal. As real activity builds up, confident claims show up because they've been earned, not because a timer ran out.

---

## The honest boundary of the guarantee

Trust comes from being precise about what is and isn't promised, so here it is plainly.

**What DocBrain guarantees:** it won't show you a confident claim it can't justify from evidence, and for ownership, hasn't passed an accuracy audit. The claims it does make are auditable, evidence-backed, and gated.

**What it doesn't claim:** to be an oracle for social facts. A team can be genuinely ambiguous. A human can declare the wrong owner. That's why DocBrain frames ownership as "inferred from real activity and accuracy-audited," clearly labeled, and never presented as absolute ground truth. The honest claim is "roughly 0% confidently wrong," which is a different and more defensible promise than "always right."

A tool that swears it's always right is asking you to trust it blind. DocBrain does the opposite. It shows you the evidence, it tells you when it doesn't know, and it proves its accuracy before it speaks. So you can check it instead of taking it on faith.

---

## See also

- [Governance](governance.md): how ownership, stewardship, and SLAs are defined and enforced
- [Autopilot](autopilot.md): how documentation gaps are detected and fixes are drafted
- [Knowledge Intelligence](knowledge-intelligence.md): how DocBrain measures coverage, freshness, and quality
- [Review Workflows](reviews.md): how human approval works
