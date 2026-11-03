"""
Generates alignments from 1<<0 to 1<<64
"""


def make_alignment(size: int):
    return f"(Align{size}, {size}),"


def group(r: range):
    print("make_alignment!(")
    for i in r:
        print(f"    {make_alignment(1 << i)}")
    print(");")


group(range(16))
print()

print('#[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]')
group(range(16, 30))

