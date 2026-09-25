declare module 'pako' {
  export class Inflate {
    constructor(options?: Record<string, unknown>)
    ended: boolean
    err: number
    msg: string
    result?: Uint8Array
    push(data: Uint8Array, flush?: boolean | number): boolean
  }
}
