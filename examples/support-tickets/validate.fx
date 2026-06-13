# Validation helpers

# Simple email check: must contain '@' and '.', and must not be empty.
exp fn valid_email s
  if s == nil
    ret false
  if str.len s == 0
    ret false
  if !(str.has s "@")
    ret false
  ret str.has s "."

# Checks that text is not empty (not nil and not an empty string)
exp fn non_empty s
  if s == nil
    ret false
  ret str.len s > 0
