/**
 * Table/view metadata from information_schema + SHOW COLUMNS (Doris).
 * Author: kejiqing
 */
import type { DorisConnection } from "./connection.js";
export interface TableMeta {
    tableSchema: string;
    tableName: string;
    tableType: string;
    engine: string;
    tableComment: string;
    tableRows: string | number | null;
    createTime: string | null;
    updateTime: string | null;
}
export interface ColumnMeta {
    field: string;
    type: string;
    null: string;
    key: string;
    default: string | null;
    extra: string;
    comment?: string;
}
export declare function getTableMeta(conn: DorisConnection, database: string, table: string): Promise<TableMeta | null>;
export declare function getColumnMeta(conn: DorisConnection, database: string, table: string): Promise<ColumnMeta[]>;
export declare function buildTableInformationText(table: TableMeta | null, columns: ColumnMeta[]): string;
