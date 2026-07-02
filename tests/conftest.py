import sys

# The lambda kernel reduces through Church numerals + Scott lists + Y in a non-tail-
# recursive host (Python), so deep FFP reductions nest many host frames. Raise the
# ceiling for correctness testing; wall-clock/depth is the delta-optimization's concern.
sys.setrecursionlimit(1_000_000)
