# For the 12 test: a module that itself imports another module (nested import).
use ./mod_math

# Reuses mod_math.add - the nested import is resolved relative to this
# module's directory.
exp fn double n -> mod_math.add n n
