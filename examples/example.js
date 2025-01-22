// Redundant comment stating the obvious
function multiply(x, y) {
    // Multiply x and y and return the result
    return x * y;
}

// Useful comment explaining the regex pattern
function isValidEmail(email) {
    // Matches standard email format: username@domain.tld
    // Username: alphanumeric, dots, underscores, or hyphens
    // Domain: alphanumeric, dots (for subdomains), and hyphens
    const emailRegex = /^[a-zA-Z0-9._-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/;
    return emailRegex.test(email);
}

// Redundant class documentation
class Car {
    // Constructor for Car class
    constructor(make, model) {
        // Set the make property
        this.make = make;
        // Set the model property
        this.model = model;
    }
} 