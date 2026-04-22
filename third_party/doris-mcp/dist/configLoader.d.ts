/**
 * Load Doris cluster config from YAML.
 * Author: kejiqing
 */
export interface ClusterConfig {
    host: string;
    port: number;
    user: string;
    password: string;
    default_database?: string;
    ssl?: boolean;
    env?: Record<string, unknown>;
    allowed_tables?: string[];
}
export interface ClustersConfig {
    clusters: Record<string, ClusterConfig>;
}
export declare function loadConfig(): ClustersConfig;
export declare function listClusterIds(config: ClustersConfig): string[];
