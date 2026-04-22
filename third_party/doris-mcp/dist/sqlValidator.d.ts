/**
 * Strict validation for Doris SQL: single statement only, SELECT / SET / EXPLAIN / SHOW.
 * Delegates parsing to Python sqlglot (scripts/validate_sql.py). Author: kejiqing
 */
export interface ValidationResult {
    ok: true;
    tableRefs: string[];
}
export interface ValidationError {
    ok: false;
    message: string;
}
export declare function validateDorisSql(sql: string): ValidationResult | ValidationError;
