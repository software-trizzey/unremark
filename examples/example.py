# Redundant comment that adds no value
def greet(name):
    # Print hello and the name
    print(f"Hello, {name}!")

# Useful comment explaining the algorithm
def fibonacci(n):
    # Using dynamic programming to avoid exponential time complexity
    # by storing previously calculated values
    if n <= 1:
        return n
    
    prev, curr = 0, 1
    for _ in range(2, n + 1):
        prev, curr = curr, prev + curr
    
    return curr

# Redundant class description
class Rectangle:
    # Constructor for Rectangle
    def __init__(self, width, height):
        self.width = width  # The width of the rectangle
        self.height = height  # The height of the rectangle 