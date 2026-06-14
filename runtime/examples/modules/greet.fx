# greet — a small module. Only `exp` names are visible from outside.

# Module-private (not exported): not visible from outside.
prefix = "Hello"

# Exported value.
exp lang = "english"

# Exported function — can reach the module-level `prefix` (closure).
exp fn hello nom -> "${prefix}, ${nom}!"
