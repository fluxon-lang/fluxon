# Helper module for the 12 test. Only `exp`-ed names are visible from outside.

# Module-private - not part of the namespace.
base = 100

exp pi = 3

exp fn add a b -> a + b

# Closure: accesses the module-level `base` (not the importer's scope).
exp fn from_base n -> base + n
