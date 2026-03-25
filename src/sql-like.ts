export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}
