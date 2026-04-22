/**
 * Format query result as MySQL CLI style.
 * Author: kejiqing
 */
export type Row = Record<string, unknown>;
export declare function formatResult(columns: string[], rows: Row[], elapsedMs?: number): string;
