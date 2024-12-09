import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, DateTimeColumn as DateTimeColumn_, Index as Index_, BigIntColumn as BigIntColumn_} from "@subsquid/typeorm-store"
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

    @StringColumn_({nullable: false})
    blockNumber!: string

    @Index_()
    @DateTimeColumn_({nullable: false})
    timestamp!: Date

    @DateTimeColumn_({nullable: true})
    completedAt!: Date | undefined | null

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

    @Column_("varchar", {length: 10, nullable: false})
    status!: Status

    @Index_()
    @StringColumn_({nullable: false})
    sender!: string

    @Index_()
    @StringColumn_({nullable: false})
    receiver!: string

    @BigIntColumn_({nullable: false})
    amount!: bigint
}
