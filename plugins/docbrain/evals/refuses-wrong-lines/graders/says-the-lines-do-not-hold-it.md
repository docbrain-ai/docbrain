---
type: regex
pattern: "not in the file|does not contain|doesn.t contain|isn.t in|is not in|not present|isn.t present|cannot anchor|can.t anchor|no such annotation|don.t hold|do not hold|doesn.t (actually |currently |yet )?(have|include|exist|contain|hold)|not (actually |currently |yet )?(there|in|applied|present|committed)|nowhere in|missing from|absent from|has not been applied|hasn.t been applied|not found in|lines? 6-7 (hold|holds|contain|contains|are|is|show|shows)"
flags: i
match: contains
target: last_message
---
