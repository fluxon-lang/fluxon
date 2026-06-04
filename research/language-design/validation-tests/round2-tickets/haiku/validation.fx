# Email and input validation helpers

fn is_valid_email email
  # Basic email validation: contains @ and .
  has_at = str.has email "@"
  has_dot = str.has email "."
  at_pos = str.find email "@"
  dot_pos = str.find email "."

  valid = has_at & has_dot & (dot_pos > at_pos)
  valid

fn validate_ticket_input email subject body
  # Returns {ok:bool, error:str or nil}

  if !email
    ret {ok:false, error:"email required"}

  if !is_valid_email email
    ret {ok:false, error:"invalid email format"}

  if !subject
    ret {ok:false, error:"subject required"}

  if !body
    ret {ok:false, error:"body required"}

  {ok:true, error:nil}

exp is_valid_email validate_ticket_input
