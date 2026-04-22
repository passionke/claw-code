/**
 * Doris connection helper.
 * Author: kejiqing
 */
import type { ClusterConfig } from "./configLoader.js";
export interface DorisConnection {
    query(sql: string): Promise<[unknown[], {
        name?: string;
    }[]]>;
    end(): Promise<void>;
}
export declare function getConnection(_clusterId: string, config: ClusterConfig, database?: string): Promise<DorisConnection>;
export declare function evictConnection(_clusterId: string, _database: string): void;
export declare function touchConnection(_clusterId: string, _database: string): void;
export declare function releaseConnection(_clusterId: string, _database: string, _conn: DorisConnection): void;
