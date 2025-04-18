import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_} from "@subsquid/typeorm-store"
import {Network} from "./_network"

@Entity_()
export class Pair {
    constructor(props?: Partial<Pair>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @StringColumn_({nullable: false})
    gearToken!: string

    @StringColumn_({nullable: false})
    ethToken!: string

    @Column_("varchar", {length: 8, nullable: false})
    tokenSupply!: Network
}
