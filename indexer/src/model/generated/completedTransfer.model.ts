import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, Index as Index_, DateTimeColumn as DateTimeColumn_, BigIntColumn as BigIntColumn_} from "@subsquid/typeorm-store"
import {Network} from "./_network"

@Entity_()
export class CompletedTransfer {
    constructor(props?: Partial<CompletedTransfer>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @Index_({unique: true})
    @StringColumn_({nullable: false})
    nonce!: string

    @Column_("varchar", {length: 8, nullable: false})
    destNetwork!: Network

    @Column_("varchar", {length: 8, nullable: true})
    srcNetwork!: Network | undefined | null

    @DateTimeColumn_({nullable: true})
    timestamp!: Date | undefined | null

    @StringColumn_({nullable: false})
    txHash!: string

    @BigIntColumn_({nullable: false})
    blockNumber!: bigint
}
