import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, BigIntColumn as BigIntColumn_, DateTimeColumn as DateTimeColumn_, Index as Index_} from "@subsquid/typeorm-store"
import {Network} from "./_network"
import {Status} from "./_status"

@Entity_()
export class Transfer {
    constructor(props?: Partial<Transfer>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @StringColumn_({nullable: false})
    txHash!: string

    @BigIntColumn_({nullable: false})
    blockNumber!: bigint

    @Index_()
    @DateTimeColumn_({nullable: false})
    timestamp!: Date

    @DateTimeColumn_({nullable: true})
    completedAt!: Date | undefined | null

    @BigIntColumn_({nullable: true})
    completedAtBlock!: bigint | undefined | null

    @StringColumn_({nullable: true})
    completedAtTxHash!: string | undefined | null

    @Index_()
    @StringColumn_({nullable: false})
    nonce!: string

    @Column_("varchar", {length: 8, nullable: false})
    sourceNetwork!: Network

    @Index_()
    @StringColumn_({nullable: false})
    source!: string

    @Column_("varchar", {length: 8, nullable: false})
    destNetwork!: Network

    @Index_()
    @StringColumn_({nullable: false})
    destination!: string

    @Column_("varchar", {length: 15, nullable: false})
    status!: Status

    @Index_()
    @StringColumn_({nullable: false})
    sender!: string

    @Index_()
    @StringColumn_({nullable: false})
    receiver!: string

    @BigIntColumn_({nullable: false})
    amount!: bigint

    @BigIntColumn_({nullable: true})
    bridgingStartedAtBlock!: bigint | undefined | null

    @StringColumn_({nullable: true})
    bridgingStartedAtMessageId!: string | undefined | null
}
