import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, Index as Index_, DateTimeColumn as DateTimeColumn_} from "@subsquid/typeorm-store"
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

    @DateTimeColumn_({nullable: true})
    timestamp!: Date | undefined | null
}
