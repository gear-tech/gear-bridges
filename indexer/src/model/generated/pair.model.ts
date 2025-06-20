import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, Index as Index_, IntColumn as IntColumn_, BooleanColumn as BooleanColumn_, BigIntColumn as BigIntColumn_} from "@subsquid/typeorm-store"
import {Network} from "./_network"

@Entity_()
export class Pair {
    constructor(props?: Partial<Pair>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @Index_()
    @StringColumn_({nullable: false})
    varaToken!: string

    @StringColumn_({nullable: false})
    varaTokenSymbol!: string

    @IntColumn_({nullable: false})
    varaTokenDecimals!: number

    @StringColumn_({nullable: false})
    varaTokenName!: string

    @Index_()
    @StringColumn_({nullable: false})
    ethToken!: string

    @StringColumn_({nullable: false})
    ethTokenSymbol!: string

    @IntColumn_({nullable: false})
    ethTokenDecimals!: number

    @StringColumn_({nullable: false})
    ethTokenName!: string

    @Column_("varchar", {length: 8, nullable: false})
    tokenSupply!: Network

    @BooleanColumn_({nullable: false})
    isRemoved!: boolean

    @BigIntColumn_({nullable: false})
    activeSinceBlock!: bigint

    @StringColumn_({nullable: true})
    upgradedTo!: string | undefined | null

    @BigIntColumn_({nullable: true})
    activeToBlock!: bigint | undefined | null

    @BooleanColumn_({nullable: false})
    isActive!: boolean
}
