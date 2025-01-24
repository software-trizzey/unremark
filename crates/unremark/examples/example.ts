// Redundant interface description
interface User {
    // The user's name
    name: string;
    // The user's age
    age: number;
    // The user's email
    email: string;
}

// Useful comment explaining the generic constraint
// T must be an object with a 'compare' method that returns a number
function sort<T extends { compare(other: T): number }>(items: T[]): T[] {
    return items.sort((a, b) => a.compare(b));
}

// Redundant function description
function calculateTotal(items: { price: number }[]): number {
    // Sum all item prices
    return items.reduce((sum, item) => sum + item.price, 0);
}

// Useful comment explaining the type guard
function isString(value: unknown): value is string {
    // Using typeof for runtime type checking
    // This is a type predicate that helps TypeScript narrow types
    return typeof value === 'string';
} 