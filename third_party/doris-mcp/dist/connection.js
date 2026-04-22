/**
 * Doris connection helper.
 * Author: kejiqing
 */
import mysql from "mysql";
function wrapConnection(raw) {
    return {
        query(sql) {
            return new Promise((resolve, reject) => {
                raw.query(sql, (err, results, fields) => {
                    if (err)
                        reject(err);
                    else {
                        let rows = [];
                        let outFields = [];
                        if (results != null && typeof results === "object" && !Array.isArray(results)) {
                            rows = [results];
                            outFields = Object.keys(results).map((name) => ({ name }));
                        }
                        else if (Array.isArray(results) && results.length > 0) {
                            const first = results[0];
                            if (results.length === 1) {
                                rows = Array.isArray(first) ? first : results;
                            }
                            else {
                                for (let i = results.length - 1; i >= 0; i--) {
                                    if (Array.isArray(results[i])) {
                                        rows = results[i];
                                        break;
                                    }
                                }
                                if (rows.length === 0 && results.length > 0 && typeof first === "object" && first !== null)
                                    rows = results;
                            }
                            if (rows.length > 0 && typeof rows[0] === "object" && rows[0] !== null) {
                                outFields = Object.keys(rows[0]).map((name) => ({ name }));
                            }
                            else if (Array.isArray(fields) && fields.length > 0) {
                                const lastFields = fields.length > 1 && Array.isArray(fields[fields.length - 1])
                                    ? fields[fields.length - 1]
                                    : fields;
                                outFields = lastFields.map((f) => ({ name: f?.name ?? "" }));
                            }
                        }
                        resolve([rows, outFields]);
                    }
                });
            });
        },
        end() {
            return new Promise((resolve, reject) => {
                raw.end((err) => (err ? reject(err) : resolve()));
            });
        },
    };
}
export async function getConnection(_clusterId, config, database) {
    const useDb = database === ""
        ? undefined
        : (database ?? config.default_database ?? undefined);
    const raw = mysql.createConnection({
        host: config.host,
        port: config.port,
        user: config.user,
        password: config.password,
        database: useDb,
        charset: "utf8",
        connectTimeout: 15000,
        multipleStatements: true,
        ...(config.ssl ? { ssl: { rejectUnauthorized: false } } : {}),
    });
    await new Promise((resolve, reject) => {
        raw.connect((err) => (err ? reject(err) : resolve()));
    });
    return wrapConnection(raw);
}
export function evictConnection(_clusterId, _database) {
    // no-op
}
export function touchConnection(_clusterId, _database) {
    // no-op
}
export function releaseConnection(_clusterId, _database, _conn) {
    // no-op
}
//# sourceMappingURL=connection.js.map