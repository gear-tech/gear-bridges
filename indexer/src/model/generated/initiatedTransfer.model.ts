import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, BigIntColumn as BigIntColumn_} from "@subsquid/typeorm-store"

@Entity_()
export class InitiatedTransfer {
    constructor(props?: Partial<InitiatedTransfer>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @StringColumn_({nullable: false})
    txHash!: string

    @BigIntColumn_({nullable: false})
    blockNumber!: bigint
}
