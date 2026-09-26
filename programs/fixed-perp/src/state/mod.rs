pub mod error;
pub mod governance;
pub mod market;
pub mod order_book;
pub mod user;
pub mod valuation;
pub mod vault;

pub use error::{StateError, StateResult};
pub use fixed_perp_math::FixedPoint;
pub use governance::{
    CircuitBreakerProposal, FeeRateProposal, GlobalGovernanceConfig, GroupMarginProposal,
    LiquidationProposal, MarginGroupGovernance, MarketGovernance, MarketMarginProposal,
    MarketRiskProposal, MarketStatusProposal, OptionalFixedPoint, OptionalU64, ProposalWindow,
    ResourceGovernance, VaultGovernance,
};
pub use market::{
    aggregate_is_closing_action, BatchOrder, ClosingAction, JupiterAdapterPayload, MakerSubaccount,
    MarginGroup, MarginRequirement, MarginRequirementType, MarketConfig, MarketGroupMembership,
    MarketState, MarketStatus, ModifyOrderIntent, PhoenixAdapterPayload, PlaceOrderBatchIntent,
    PlaceOrderIntent, PlaceOrderResult, PortfolioHealth, PortfolioValuation, VenueAdapterConfig,
    VenueAdapterKind, VenueLayout, ADAPTER_JUPITER, ADAPTER_PHOENIX, JUPITER_PAYLOAD_RESERVED,
    MARGIN_GROUP_RESERVED, MARKET_ACTIVE, MARKET_CLOSE_ONLY, MARKET_CONFIG_RESERVED,
    MARKET_INITIALIZING, MARKET_MATURED, MARKET_PAUSED, MARKET_STATE_RESERVED,
    PHOENIX_PAYLOAD_RESERVED, SECONDS_PER_YEAR, VENUE_CONFIG_RESERVED,
};
pub use order_book::{
    HierarchicalBitmap, OrderBook, OrderId, OrderNode, Side, TickMetadata, TimeInForce,
    BOOK_CAPACITY, FREE_INDEX, ORDER_BOOK_RESERVED, ORDER_NODE_RESERVED, TICK_DIRECTORY_LEN,
    TICK_METADATA_RESERVED,
};
pub use user::{
    MarketSlot, OrderRef, OrderRiskAggregate, Subaccount, SubaccountStatus, MARKET_SLOT_RESERVED,
    MAX_MARKETS, MAX_ORDERS, SUBACCOUNT_ACTIVE, SUBACCOUNT_RESERVED,
};
pub use valuation::{PriceObservation, ValuationFactors};
pub use vault::{CollateralVault, COLLATERAL_VAULT_RESERVED};
