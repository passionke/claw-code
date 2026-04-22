/**
 * Load Doris cluster config from YAML.
 * Author: kejiqing
 */
import fs from "node:fs";
import path from "node:path";
import yaml from "js-yaml";
function isPrivilegedDbUser(user) {
    const normalized = user.trim().toLowerCase();
    return normalized === "root" || normalized === "admin";
}
function allowPrivilegedUserByCluster(clusterId) {
    const normalizedId = clusterId.trim().toLowerCase();
    return normalizedId.includes("dev") || normalizedId.includes("local");
}
function getConfigPath() {
    const envPath = process.env.DORIS_CONFIG;
    if (envPath)
        return envPath;
    return path.join(process.cwd(), "config", "doris_clusters.yaml");
}
function normalizeAllowedTable(value) {
    return String(value ?? "")
        .trim()
        .replace(/[`"]/g, "")
        .replace(/\s+/g, "")
        .toLowerCase();
}
export function loadConfig() {
    const configPath = getConfigPath();
    if (!fs.existsSync(configPath)) {
        throw new Error(`Doris 集群配置文件不存在: ${configPath}。请设置 DORIS_CONFIG 或于 config/doris_clusters.yaml 配置集群。`);
    }
    const content = fs.readFileSync(configPath, "utf8");
    const raw = yaml.load(content);
    if (!raw || typeof raw !== "object" || !("clusters" in raw)) {
        throw new Error(`配置文件格式错误: ${configPath}，需包含 clusters 对象。`);
    }
    const clusters = raw.clusters;
    if (clusters != null && typeof clusters !== "object") {
        throw new Error(`配置文件 ${configPath} 中 clusters 必须为对象。`);
    }
    const clustersObj = clusters && typeof clusters === "object" ? clusters : {};
    const result = {};
    for (const [id, c] of Object.entries(clustersObj)) {
        if (!c || typeof c !== "object") {
            throw new Error(`集群 ${id} 配置必须为对象。`);
        }
        const obj = c;
        const port = typeof obj.port === "number" ? obj.port : Number(obj.port);
        if (typeof obj.host !== "string" || Number.isNaN(port)) {
            throw new Error(`集群 ${id} 需提供 host(string) 与 port(number)。`);
        }
        const user = typeof obj.user === "string" ? obj.user : "";
        if (isPrivilegedDbUser(user) && !allowPrivilegedUserByCluster(id)) {
            throw new Error(`集群 ${id} 配置非法：仅集群名包含 dev/local 时允许使用 root/admin，当前 user=${user || "(empty)"}。`);
        }
        result[id] = {
            host: obj.host,
            port,
            user,
            password: obj.password ?? "",
            default_database: typeof obj.default_database === "string"
                ? obj.default_database
                : undefined,
            ssl: obj.ssl === true,
            env: obj.env != null && typeof obj.env === "object" && !Array.isArray(obj.env)
                ? obj.env
                : undefined,
            allowed_tables: Array.isArray(obj.allowed_tables)
                ? obj.allowed_tables.map(normalizeAllowedTable).filter(Boolean)
                : undefined,
        };
    }
    return { clusters: result };
}
export function listClusterIds(config) {
    return Object.keys(config.clusters);
}
//# sourceMappingURL=configLoader.js.map