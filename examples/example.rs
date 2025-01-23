// This is a redundant comment that adds no value
use std::collections::HashMap;

// Function to add two numbers together
fn add(a: i32, b: i32) -> i32 {
    // Return the sum of a and b
    a + b
}

// A struct to store user data
struct User {
    // The user's name
    name: String,
    // The user's age in years
    age: u32,
}

// Implementation block for User struct
impl User {
    // Constructor for User
    fn new(name: String, age: u32) -> Self {
        // Create a new instance
        Self { name, age }
    }

    // This method prints user information
    fn print_info(&self) {
        // Print the formatted string
        println!("Name: {}, Age: {}", self.name, self.age);
    }
}

/// This function uses dynamic programming to calculate Fibonacci numbers
/// efficiently by storing previously computed values in a HashMap to
/// avoid redundant calculations.
fn fibonacci(n: u64) -> u64 {
    let mut memo = HashMap::new();
    memo.insert(0, 0);
    memo.insert(1, 1);
    
    // Helper function for recursive calculation
    fn fib_helper(n: u64, memo: &mut HashMap<u64, u64>) -> u64 {
        // Check if we've already calculated this value
        if let Some(&result) = memo.get(&n) {
            return result;
        }
        
        // Calculate new value and store it
        let result = fib_helper(n - 1, memo) + fib_helper(n - 2, memo);
        memo.insert(n, result);
        result
    }
    
    fib_helper(n, &mut memo)
}

fn main() {
    let user = User::new("Alice".to_string(), 30);
    user.print_info();
    
    println!("Fibonacci(10) = {}", fibonacci(10));
    println!("Sum: {}", add(5, 3)); // This is a redundant comment
} 