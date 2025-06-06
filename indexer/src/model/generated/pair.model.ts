import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, BooleanColumn as BooleanColumn_} from "@subsquid/typeorm-store"
import {Network} from "./_network"

@Entity_()
export class Pair {
    constructor(props?: Partial<Pair>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @StringColumn_({nullable: false})
    varaToken!: string

    @StringColumn_({nullable: false})
    varaTokenSymbol!: string

    @StringColumn_({nullable: false})
    ethToken!: string

    @StringColumn_({nullable: false})
    ethTokenSymbol!: string

    @Column_("varchar", {length: 8, nullable: false})
    tokenSupply!: Network

    @BooleanColumn_({nullable: false})
    isRemoved!: boolean
}
