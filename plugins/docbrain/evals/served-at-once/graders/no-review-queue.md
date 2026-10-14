---
type: regex
pattern: "review queue|queued|wait(s|ing)? (for|in) (a |the )?(human )?review"
flags: i
match: not_contains
target: last_message
---
