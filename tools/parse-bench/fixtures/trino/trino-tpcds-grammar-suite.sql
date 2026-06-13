-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q66.sql
SELECT
  "w_warehouse_name"
, "w_warehouse_sq_ft"
, "w_city"
, "w_county"
, "w_state"
, "w_country"
, "ship_carriers"
, "year"
, "sum"("jan_sales") "jan_sales"
, "sum"("feb_sales") "feb_sales"
, "sum"("mar_sales") "mar_sales"
, "sum"("apr_sales") "apr_sales"
, "sum"("may_sales") "may_sales"
, "sum"("jun_sales") "jun_sales"
, "sum"("jul_sales") "jul_sales"
, "sum"("aug_sales") "aug_sales"
, "sum"("sep_sales") "sep_sales"
, "sum"("oct_sales") "oct_sales"
, "sum"("nov_sales") "nov_sales"
, "sum"("dec_sales") "dec_sales"
, "sum"(("jan_sales" / "w_warehouse_sq_ft")) "jan_sales_per_sq_foot"
, "sum"(("feb_sales" / "w_warehouse_sq_ft")) "feb_sales_per_sq_foot"
, "sum"(("mar_sales" / "w_warehouse_sq_ft")) "mar_sales_per_sq_foot"
, "sum"(("apr_sales" / "w_warehouse_sq_ft")) "apr_sales_per_sq_foot"
, "sum"(("may_sales" / "w_warehouse_sq_ft")) "may_sales_per_sq_foot"
, "sum"(("jun_sales" / "w_warehouse_sq_ft")) "jun_sales_per_sq_foot"
, "sum"(("jul_sales" / "w_warehouse_sq_ft")) "jul_sales_per_sq_foot"
, "sum"(("aug_sales" / "w_warehouse_sq_ft")) "aug_sales_per_sq_foot"
, "sum"(("sep_sales" / "w_warehouse_sq_ft")) "sep_sales_per_sq_foot"
, "sum"(("oct_sales" / "w_warehouse_sq_ft")) "oct_sales_per_sq_foot"
, "sum"(("nov_sales" / "w_warehouse_sq_ft")) "nov_sales_per_sq_foot"
, "sum"(("dec_sales" / "w_warehouse_sq_ft")) "dec_sales_per_sq_foot"
, "sum"("jan_net") "jan_net"
, "sum"("feb_net") "feb_net"
, "sum"("mar_net") "mar_net"
, "sum"("apr_net") "apr_net"
, "sum"("may_net") "may_net"
, "sum"("jun_net") "jun_net"
, "sum"("jul_net") "jul_net"
, "sum"("aug_net") "aug_net"
, "sum"("sep_net") "sep_net"
, "sum"("oct_net") "oct_net"
, "sum"("nov_net") "nov_net"
, "sum"("dec_net") "dec_net"
FROM
(
      SELECT
        "w_warehouse_name"
      , "w_warehouse_sq_ft"
      , "w_city"
      , "w_county"
      , "w_state"
      , "w_country"
      , "concat"("concat"('DHL', ','), 'BARIAN') "ship_carriers"
      , "d_year" "YEAR"
      , "sum"((CASE WHEN ("d_moy" = 1) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "jan_sales"
      , "sum"((CASE WHEN ("d_moy" = 2) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "feb_sales"
      , "sum"((CASE WHEN ("d_moy" = 3) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "mar_sales"
      , "sum"((CASE WHEN ("d_moy" = 4) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "apr_sales"
      , "sum"((CASE WHEN ("d_moy" = 5) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "may_sales"
      , "sum"((CASE WHEN ("d_moy" = 6) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "jun_sales"
      , "sum"((CASE WHEN ("d_moy" = 7) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "jul_sales"
      , "sum"((CASE WHEN ("d_moy" = 8) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "aug_sales"
      , "sum"((CASE WHEN ("d_moy" = 9) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "sep_sales"
      , "sum"((CASE WHEN ("d_moy" = 10) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "oct_sales"
      , "sum"((CASE WHEN ("d_moy" = 11) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "nov_sales"
      , "sum"((CASE WHEN ("d_moy" = 12) THEN ("ws_ext_sales_price" * "ws_quantity") ELSE 0 END)) "dec_sales"
      , "sum"((CASE WHEN ("d_moy" = 1) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "jan_net"
      , "sum"((CASE WHEN ("d_moy" = 2) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "feb_net"
      , "sum"((CASE WHEN ("d_moy" = 3) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "mar_net"
      , "sum"((CASE WHEN ("d_moy" = 4) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "apr_net"
      , "sum"((CASE WHEN ("d_moy" = 5) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "may_net"
      , "sum"((CASE WHEN ("d_moy" = 6) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "jun_net"
      , "sum"((CASE WHEN ("d_moy" = 7) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "jul_net"
      , "sum"((CASE WHEN ("d_moy" = 8) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "aug_net"
      , "sum"((CASE WHEN ("d_moy" = 9) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "sep_net"
      , "sum"((CASE WHEN ("d_moy" = 10) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "oct_net"
      , "sum"((CASE WHEN ("d_moy" = 11) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "nov_net"
      , "sum"((CASE WHEN ("d_moy" = 12) THEN ("ws_net_paid" * "ws_quantity") ELSE 0 END)) "dec_net"
      FROM
        catalog.schema.web_sales
      , catalog.schema.warehouse
      , catalog.schema.date_dim
      , catalog.schema.time_dim
      , catalog.schema.ship_mode
      WHERE ("ws_warehouse_sk" = "w_warehouse_sk")
         AND ("ws_sold_date_sk" = "d_date_sk")
         AND ("ws_sold_time_sk" = "t_time_sk")
         AND ("ws_ship_mode_sk" = "sm_ship_mode_sk")
         AND ("d_year" = 2001)
         AND ("t_time" BETWEEN 30838 AND (30838 + 28800))
         AND ("sm_carrier" IN ('DHL'      , 'BARIAN'))
      GROUP BY "w_warehouse_name", "w_warehouse_sq_ft", "w_city", "w_county", "w_state", "w_country", "d_year"
   UNION ALL
      SELECT
        "w_warehouse_name"
      , "w_warehouse_sq_ft"
      , "w_city"
      , "w_county"
      , "w_state"
      , "w_country"
      , "concat"("concat"('DHL', ','), 'BARIAN') "ship_carriers"
      , "d_year" "YEAR"
      , "sum"((CASE WHEN ("d_moy" = 1) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "jan_sales"
      , "sum"((CASE WHEN ("d_moy" = 2) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "feb_sales"
      , "sum"((CASE WHEN ("d_moy" = 3) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "mar_sales"
      , "sum"((CASE WHEN ("d_moy" = 4) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "apr_sales"
      , "sum"((CASE WHEN ("d_moy" = 5) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "may_sales"
      , "sum"((CASE WHEN ("d_moy" = 6) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "jun_sales"
      , "sum"((CASE WHEN ("d_moy" = 7) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "jul_sales"
      , "sum"((CASE WHEN ("d_moy" = 8) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "aug_sales"
      , "sum"((CASE WHEN ("d_moy" = 9) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "sep_sales"
      , "sum"((CASE WHEN ("d_moy" = 10) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "oct_sales"
      , "sum"((CASE WHEN ("d_moy" = 11) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "nov_sales"
      , "sum"((CASE WHEN ("d_moy" = 12) THEN ("cs_sales_price" * "cs_quantity") ELSE 0 END)) "dec_sales"
      , "sum"((CASE WHEN ("d_moy" = 1) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "jan_net"
      , "sum"((CASE WHEN ("d_moy" = 2) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "feb_net"
      , "sum"((CASE WHEN ("d_moy" = 3) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "mar_net"
      , "sum"((CASE WHEN ("d_moy" = 4) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "apr_net"
      , "sum"((CASE WHEN ("d_moy" = 5) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "may_net"
      , "sum"((CASE WHEN ("d_moy" = 6) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "jun_net"
      , "sum"((CASE WHEN ("d_moy" = 7) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "jul_net"
      , "sum"((CASE WHEN ("d_moy" = 8) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "aug_net"
      , "sum"((CASE WHEN ("d_moy" = 9) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "sep_net"
      , "sum"((CASE WHEN ("d_moy" = 10) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "oct_net"
      , "sum"((CASE WHEN ("d_moy" = 11) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "nov_net"
      , "sum"((CASE WHEN ("d_moy" = 12) THEN ("cs_net_paid_inc_tax" * "cs_quantity") ELSE 0 END)) "dec_net"
      FROM
        catalog.schema.catalog_sales
      , catalog.schema.warehouse
      , catalog.schema.date_dim
      , catalog.schema.time_dim
      , catalog.schema.ship_mode
      WHERE ("cs_warehouse_sk" = "w_warehouse_sk")
         AND ("cs_sold_date_sk" = "d_date_sk")
         AND ("cs_sold_time_sk" = "t_time_sk")
         AND ("cs_ship_mode_sk" = "sm_ship_mode_sk")
         AND ("d_year" = 2001)
         AND ("t_time" BETWEEN 30838 AND (30838 + 28800))
         AND ("sm_carrier" IN ('DHL'      , 'BARIAN'))
      GROUP BY "w_warehouse_name", "w_warehouse_sq_ft", "w_city", "w_county", "w_state", "w_country", "d_year"
   )  x
GROUP BY "w_warehouse_name", "w_warehouse_sq_ft", "w_city", "w_county", "w_state", "w_country", "ship_carriers", "year"
ORDER BY "w_warehouse_name" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q05.sql
WITH
  ssr AS (
   SELECT
     "s_store_id"
   , "sum"("sales_price") "sales"
   , "sum"("profit") "profit"
   , "sum"("return_amt") "returns"
   , "sum"("net_loss") "profit_loss"
   FROM
     (
      SELECT
        "ss_store_sk" "store_sk"
      , "ss_sold_date_sk" "date_sk"
      , "ss_ext_sales_price" "sales_price"
      , "ss_net_profit" "profit"
      , CAST(0 AS DECIMAL(7,2)) "return_amt"
      , CAST(0 AS DECIMAL(7,2)) "net_loss"
      FROM
        catalog.schema.store_sales
UNION ALL       SELECT
        "sr_store_sk" "store_sk"
      , "sr_returned_date_sk" "date_sk"
      , CAST(0 AS DECIMAL(7,2)) "sales_price"
      , CAST(0 AS DECIMAL(7,2)) "profit"
      , "sr_return_amt" "return_amt"
      , "sr_net_loss" "net_loss"
      FROM
        catalog.schema.store_returns
   )  salesreturns
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '14' DAY))
      AND ("store_sk" = "s_store_sk")
   GROUP BY "s_store_id"
) 
, csr AS (
   SELECT
     "cp_catalog_page_id"
   , "sum"("sales_price") "sales"
   , "sum"("profit") "profit"
   , "sum"("return_amt") "returns"
   , "sum"("net_loss") "profit_loss"
   FROM
     (
      SELECT
        "cs_catalog_page_sk" "page_sk"
      , "cs_sold_date_sk" "date_sk"
      , "cs_ext_sales_price" "sales_price"
      , "cs_net_profit" "profit"
      , CAST(0 AS DECIMAL(7,2)) "return_amt"
      , CAST(0 AS DECIMAL(7,2)) "net_loss"
      FROM
        catalog.schema.catalog_sales
UNION ALL       SELECT
        "cr_catalog_page_sk" "page_sk"
      , "cr_returned_date_sk" "date_sk"
      , CAST(0 AS DECIMAL(7,2)) "sales_price"
      , CAST(0 AS DECIMAL(7,2)) "profit"
      , "cr_return_amount" "return_amt"
      , "cr_net_loss" "net_loss"
      FROM
        catalog.schema.catalog_returns
   )  salesreturns
   , catalog.schema.date_dim
   , catalog.schema.catalog_page
   WHERE ("date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '14' DAY))
      AND ("page_sk" = "cp_catalog_page_sk")
   GROUP BY "cp_catalog_page_id"
) 
, wsr AS (
   SELECT
     "web_site_id"
   , "sum"("sales_price") "sales"
   , "sum"("profit") "profit"
   , "sum"("return_amt") "returns"
   , "sum"("net_loss") "profit_loss"
   FROM
     (
      SELECT
        "ws_web_site_sk" "wsr_web_site_sk"
      , "ws_sold_date_sk" "date_sk"
      , "ws_ext_sales_price" "sales_price"
      , "ws_net_profit" "profit"
      , CAST(0 AS DECIMAL(7,2)) "return_amt"
      , CAST(0 AS DECIMAL(7,2)) "net_loss"
      FROM
        catalog.schema.web_sales
UNION ALL       SELECT
        "ws_web_site_sk" "wsr_web_site_sk"
      , "wr_returned_date_sk" "date_sk"
      , CAST(0 AS DECIMAL(7,2)) "sales_price"
      , CAST(0 AS DECIMAL(7,2)) "profit"
      , "wr_return_amt" "return_amt"
      , "wr_net_loss" "net_loss"
      FROM
        (catalog.schema.web_returns
      LEFT JOIN catalog.schema.web_sales ON ("wr_item_sk" = "ws_item_sk")
         AND ("wr_order_number" = "ws_order_number"))
   )  salesreturns
   , catalog.schema.date_dim
   , catalog.schema.web_site
   WHERE ("date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '14' DAY))
      AND ("wsr_web_site_sk" = "web_site_sk")
   GROUP BY "web_site_id"
) 
SELECT
  "channel"
, "id"
, "sum"("sales") "sales"
, "sum"("returns") "returns"
, "sum"("profit") "profit"
FROM
  (
   SELECT
     'store channel' "channel"
   , "concat"('store', "s_store_id") "id"
   , "sales"
   , "returns"
   , ("profit" - "profit_loss") "profit"
   FROM
     ssr
UNION ALL    SELECT
     'catalog channel' "channel"
   , "concat"('catalog_page', "cp_catalog_page_id") "id"
   , "sales"
   , "returns"
   , ("profit" - "profit_loss") "profit"
   FROM
     csr
UNION ALL    SELECT
     'web channel' "channel"
   , "concat"('web_site', "web_site_id") "id"
   , "sales"
   , "returns"
   , ("profit" - "profit_loss") "profit"
   FROM
     wsr
)  x
GROUP BY ROLLUP (channel, id)
ORDER BY "channel" ASC, "id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q77.sql
WITH
  ss AS (
   SELECT
     "s_store_sk"
   , "sum"("ss_ext_sales_price") "sales"
   , "sum"("ss_net_profit") "profit"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("ss_store_sk" = "s_store_sk")
   GROUP BY "s_store_sk"
) 
, sr AS (
   SELECT
     "s_store_sk"
   , "sum"("sr_return_amt") "returns"
   , "sum"("sr_net_loss") "profit_loss"
   FROM
     catalog.schema.store_returns
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("sr_returned_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("sr_store_sk" = "s_store_sk")
   GROUP BY "s_store_sk"
) 
, cs AS (
   SELECT
     "cs_call_center_sk"
   , "sum"("cs_ext_sales_price") "sales"
   , "sum"("cs_net_profit") "profit"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   WHERE ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
   GROUP BY "cs_call_center_sk"
) 
, cr AS (
   SELECT
     "cr_call_center_sk"
   , "sum"("cr_return_amount") "returns"
   , "sum"("cr_net_loss") "profit_loss"
   FROM
     catalog.schema.catalog_returns
   , catalog.schema.date_dim
   WHERE ("cr_returned_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
   GROUP BY "cr_call_center_sk"
) 
, ws AS (
   SELECT
     "wp_web_page_sk"
   , "sum"("ws_ext_sales_price") "sales"
   , "sum"("ws_net_profit") "profit"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   , catalog.schema.web_page
   WHERE ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("ws_web_page_sk" = "wp_web_page_sk")
   GROUP BY "wp_web_page_sk"
) 
, wr AS (
   SELECT
     "wp_web_page_sk"
   , "sum"("wr_return_amt") "returns"
   , "sum"("wr_net_loss") "profit_loss"
   FROM
     catalog.schema.web_returns
   , catalog.schema.date_dim
   , catalog.schema.web_page
   WHERE ("wr_returned_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("wr_web_page_sk" = "wp_web_page_sk")
   GROUP BY "wp_web_page_sk"
) 
SELECT
  "channel"
, "id"
, "sum"("sales") "sales"
, "sum"("returns") "returns"
, "sum"("profit") "profit"
FROM
  (
   SELECT
     'store channel' "channel"
   , "ss"."s_store_sk" "id"
   , "sales"
   , COALESCE("returns", 0) "returns"
   , ("profit" - COALESCE("profit_loss", 0)) "profit"
   FROM
     (ss
   LEFT JOIN sr ON ("ss"."s_store_sk" = "sr"."s_store_sk"))
UNION ALL    SELECT
     'catalog channel' "channel"
   , "cs_call_center_sk" "id"
   , "sales"
   , "returns"
   , ("profit" - "profit_loss") "profit"
   FROM
     cs
   , cr
UNION ALL    SELECT
     'web channel' "channel"
   , "ws"."wp_web_page_sk" "id"
   , "sales"
   , COALESCE("returns", 0) "returns"
   , ("profit" - COALESCE("profit_loss", 0)) "profit"
   FROM
     (ws
   LEFT JOIN wr ON ("ws"."wp_web_page_sk" = "wr"."wp_web_page_sk"))
)  x
GROUP BY ROLLUP (channel, id)
ORDER BY "channel" ASC, "id" ASC, "sales" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q83.sql
WITH
  sr_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("sr_return_quantity") "sr_item_qty"
   FROM
     catalog.schema.store_returns
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("sr_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" IN (
         SELECT "d_week_seq"
         FROM
           catalog.schema.date_dim
         WHERE ("d_date" IN (CAST('2000-06-30' AS DATE)         , CAST('2000-09-27' AS DATE)         , CAST('2000-11-17' AS DATE)))
      ))
   ))
      AND ("sr_returned_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
, cr_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("cr_return_quantity") "cr_item_qty"
   FROM
     catalog.schema.catalog_returns
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("cr_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" IN (
         SELECT "d_week_seq"
         FROM
           catalog.schema.date_dim
         WHERE ("d_date" IN (CAST('2000-06-30' AS DATE)         , CAST('2000-09-27' AS DATE)         , CAST('2000-11-17' AS DATE)))
      ))
   ))
      AND ("cr_returned_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
, wr_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("wr_return_quantity") "wr_item_qty"
   FROM
     catalog.schema.web_returns
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("wr_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" IN (
         SELECT "d_week_seq"
         FROM
           catalog.schema.date_dim
         WHERE ("d_date" IN (CAST('2000-06-30' AS DATE)         , CAST('2000-09-27' AS DATE)         , CAST('2000-11-17' AS DATE)))
      ))
   ))
      AND ("wr_returned_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
SELECT
  "sr_items"."item_id"
, "sr_item_qty"
, CAST(((("sr_item_qty" / ((CAST("sr_item_qty" AS DECIMAL(9,4)) + "cr_item_qty") + "wr_item_qty")) / DECIMAL '3.0') * 100) AS DECIMAL(7,2)) "sr_dev"
, "cr_item_qty"
, CAST(((("cr_item_qty" / ((CAST("sr_item_qty" AS DECIMAL(9,4)) + "cr_item_qty") + "wr_item_qty")) / DECIMAL '3.0') * 100) AS DECIMAL(7,2)) "cr_dev"
, "wr_item_qty"
, CAST(((("wr_item_qty" / ((CAST("sr_item_qty" AS DECIMAL(9,4)) + "cr_item_qty") + "wr_item_qty")) / DECIMAL '3.0') * 100) AS DECIMAL(7,2)) "wr_dev"
, ((("sr_item_qty" + "cr_item_qty") + "wr_item_qty") / DECIMAL '3.00') "average"
FROM
  sr_items
, cr_items
, wr_items
WHERE ("sr_items"."item_id" = "cr_items"."item_id")
   AND ("sr_items"."item_id" = "wr_items"."item_id")
ORDER BY "sr_items"."item_id" ASC, "sr_item_qty" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q28.sql
SELECT *
FROM
  (
   SELECT
     "avg"("ss_list_price") "b1_lp"
   , "count"("ss_list_price") "b1_cnt"
   , "count"(DISTINCT "ss_list_price") "b1_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 0 AND 5)
      AND (("ss_list_price" BETWEEN 8 AND (8 + 10))
         OR ("ss_coupon_amt" BETWEEN 459 AND (459 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 57 AND (57 + 20)))
)  b1
, (
   SELECT
     "avg"("ss_list_price") "b2_lp"
   , "count"("ss_list_price") "b2_cnt"
   , "count"(DISTINCT "ss_list_price") "b2_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 6 AND 10)
      AND (("ss_list_price" BETWEEN 90 AND (90 + 10))
         OR ("ss_coupon_amt" BETWEEN 2323 AND (2323 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 31 AND (31 + 20)))
)  b2
, (
   SELECT
     "avg"("ss_list_price") "b3_lp"
   , "count"("ss_list_price") "b3_cnt"
   , "count"(DISTINCT "ss_list_price") "b3_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 11 AND 15)
      AND (("ss_list_price" BETWEEN 142 AND (142 + 10))
         OR ("ss_coupon_amt" BETWEEN 12214 AND (12214 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 79 AND (79 + 20)))
)  b3
, (
   SELECT
     "avg"("ss_list_price") "b4_lp"
   , "count"("ss_list_price") "b4_cnt"
   , "count"(DISTINCT "ss_list_price") "b4_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 16 AND 20)
      AND (("ss_list_price" BETWEEN 135 AND (135 + 10))
         OR ("ss_coupon_amt" BETWEEN 6071 AND (6071 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 38 AND (38 + 20)))
)  b4
, (
   SELECT
     "avg"("ss_list_price") "b5_lp"
   , "count"("ss_list_price") "b5_cnt"
   , "count"(DISTINCT "ss_list_price") "b5_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 21 AND 25)
      AND (("ss_list_price" BETWEEN 122 AND (122 + 10))
         OR ("ss_coupon_amt" BETWEEN 836 AND (836 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 17 AND (17 + 20)))
)  b5
, (
   SELECT
     "avg"("ss_list_price") "b6_lp"
   , "count"("ss_list_price") "b6_cnt"
   , "count"(DISTINCT "ss_list_price") "b6_cntd"
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 26 AND 30)
      AND (("ss_list_price" BETWEEN 154 AND (154 + 10))
         OR ("ss_coupon_amt" BETWEEN 7326 AND (7326 + 1000))
         OR ("ss_wholesale_cost" BETWEEN 7 AND (7 + 20)))
)  b6
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q49.sql
SELECT
  'web' "channel"
, "web"."item"
, "web"."return_ratio"
, "web"."return_rank"
, "web"."currency_rank"
FROM
  (
   SELECT
     "item"
   , "return_ratio"
   , "currency_ratio"
   , "rank"() OVER (ORDER BY "return_ratio" ASC) "return_rank"
   , "rank"() OVER (ORDER BY "currency_ratio" ASC) "currency_rank"
   FROM
     (
      SELECT
        "ws"."ws_item_sk" "item"
      , (CAST("sum"(COALESCE("wr"."wr_return_quantity", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("ws"."ws_quantity", 0)) AS DECIMAL(15,4))) "return_ratio"
      , (CAST("sum"(COALESCE("wr"."wr_return_amt", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("ws"."ws_net_paid", 0)) AS DECIMAL(15,4))) "currency_ratio"
      FROM
        (catalog.schema.web_sales ws
      LEFT JOIN catalog.schema.web_returns wr ON ("ws"."ws_order_number" = "wr"."wr_order_number")
         AND ("ws"."ws_item_sk" = "wr"."wr_item_sk"))
      , catalog.schema.date_dim
      WHERE ("wr"."wr_return_amt" > 10000)
         AND ("ws"."ws_net_profit" > 1)
         AND ("ws"."ws_net_paid" > 0)
         AND ("ws"."ws_quantity" > 0)
         AND ("ws_sold_date_sk" = "d_date_sk")
         AND ("d_year" = 2001)
         AND ("d_moy" = 12)
      GROUP BY "ws"."ws_item_sk"
   )  in_web
)  web
WHERE ("web"."return_rank" <= 10)
   OR ("web"."currency_rank" <= 10)
UNION SELECT
  'catalog' "channel"
, "catalog"."item"
, "catalog"."return_ratio"
, "catalog"."return_rank"
, "catalog"."currency_rank"
FROM
  (
   SELECT
     "item"
   , "return_ratio"
   , "currency_ratio"
   , "rank"() OVER (ORDER BY "return_ratio" ASC) "return_rank"
   , "rank"() OVER (ORDER BY "currency_ratio" ASC) "currency_rank"
   FROM
     (
      SELECT
        "cs"."cs_item_sk" "item"
      , (CAST("sum"(COALESCE("cr"."cr_return_quantity", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("cs"."cs_quantity", 0)) AS DECIMAL(15,4))) "return_ratio"
      , (CAST("sum"(COALESCE("cr"."cr_return_amount", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("cs"."cs_net_paid", 0)) AS DECIMAL(15,4))) "currency_ratio"
      FROM
        (catalog.schema.catalog_sales cs
      LEFT JOIN catalog.schema.catalog_returns cr ON ("cs"."cs_order_number" = "cr"."cr_order_number")
         AND ("cs"."cs_item_sk" = "cr"."cr_item_sk"))
      , catalog.schema.date_dim
      WHERE ("cr"."cr_return_amount" > 10000)
         AND ("cs"."cs_net_profit" > 1)
         AND ("cs"."cs_net_paid" > 0)
         AND ("cs"."cs_quantity" > 0)
         AND ("cs_sold_date_sk" = "d_date_sk")
         AND ("d_year" = 2001)
         AND ("d_moy" = 12)
      GROUP BY "cs"."cs_item_sk"
   )  in_cat
)  "CATALOG"
WHERE ("catalog"."return_rank" <= 10)
   OR ("catalog"."currency_rank" <= 10)
UNION SELECT
  'store' "channel"
, "store"."item"
, "store"."return_ratio"
, "store"."return_rank"
, "store"."currency_rank"
FROM
  (
   SELECT
     "item"
   , "return_ratio"
   , "currency_ratio"
   , "rank"() OVER (ORDER BY "return_ratio" ASC) "return_rank"
   , "rank"() OVER (ORDER BY "currency_ratio" ASC) "currency_rank"
   FROM
     (
      SELECT
        "sts"."ss_item_sk" "item"
      , (CAST("sum"(COALESCE("sr"."sr_return_quantity", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("sts"."ss_quantity", 0)) AS DECIMAL(15,4))) "return_ratio"
      , (CAST("sum"(COALESCE("sr"."sr_return_amt", 0)) AS DECIMAL(15,4)) / CAST("sum"(COALESCE("sts"."ss_net_paid", 0)) AS DECIMAL(15,4))) "currency_ratio"
      FROM
        (catalog.schema.store_sales sts
      LEFT JOIN catalog.schema.store_returns sr ON ("sts"."ss_ticket_number" = "sr"."sr_ticket_number")
         AND ("sts"."ss_item_sk" = "sr"."sr_item_sk"))
      , catalog.schema.date_dim
      WHERE ("sr"."sr_return_amt" > 10000)
         AND ("sts"."ss_net_profit" > 1)
         AND ("sts"."ss_net_paid" > 0)
         AND ("sts"."ss_quantity" > 0)
         AND ("ss_sold_date_sk" = "d_date_sk")
         AND ("d_year" = 2001)
         AND ("d_moy" = 12)
      GROUP BY "sts"."ss_item_sk"
   )  in_store
)  store
WHERE ("store"."return_rank" <= 10)
   OR ("store"."currency_rank" <= 10)
ORDER BY 1 ASC, 4 ASC, 5 ASC, 2 ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q58.sql
WITH
  ss_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("ss_ext_sales_price") "ss_item_rev"
   FROM
     catalog.schema.store_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("ss_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" = (
            SELECT "d_week_seq"
            FROM
              catalog.schema.date_dim
            WHERE ("d_date" = CAST('2000-01-03' AS DATE))
         ))
   ))
      AND ("ss_sold_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
, cs_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("cs_ext_sales_price") "cs_item_rev"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("cs_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" = (
            SELECT "d_week_seq"
            FROM
              catalog.schema.date_dim
            WHERE ("d_date" = CAST('2000-01-03' AS DATE))
         ))
   ))
      AND ("cs_sold_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
, ws_items AS (
   SELECT
     "i_item_id" "item_id"
   , "sum"("ws_ext_sales_price") "ws_item_rev"
   FROM
     catalog.schema.web_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("ws_item_sk" = "i_item_sk")
      AND ("d_date" IN (
      SELECT "d_date"
      FROM
        catalog.schema.date_dim
      WHERE ("d_week_seq" = (
            SELECT "d_week_seq"
            FROM
              catalog.schema.date_dim
            WHERE ("d_date" = CAST('2000-01-03' AS DATE))
         ))
   ))
      AND ("ws_sold_date_sk" = "d_date_sk")
   GROUP BY "i_item_id"
) 
SELECT
  "ss_items"."item_id"
, "ss_item_rev"
, CAST(((("ss_item_rev" / ((CAST("ss_item_rev" AS DECIMAL(16,7)) + "cs_item_rev") + "ws_item_rev")) / 3) * 100) AS DECIMAL(7,2)) "ss_dev"
, "cs_item_rev"
, CAST(((("cs_item_rev" / ((CAST("ss_item_rev" AS DECIMAL(16,7)) + "cs_item_rev") + "ws_item_rev")) / 3) * 100) AS DECIMAL(7,2)) "cs_dev"
, "ws_item_rev"
, CAST(((("ws_item_rev" / ((CAST("ss_item_rev" AS DECIMAL(16,7)) + "cs_item_rev") + "ws_item_rev")) / 3) * 100) AS DECIMAL(7,2)) "ws_dev"
, ((("ss_item_rev" + "cs_item_rev") + "ws_item_rev") / 3) "average"
FROM
  ss_items
, cs_items
, ws_items
WHERE ("ss_items"."item_id" = "cs_items"."item_id")
   AND ("ss_items"."item_id" = "ws_items"."item_id")
   AND ("ss_item_rev" BETWEEN (DECIMAL '0.9' * "cs_item_rev") AND (DECIMAL '1.1' * "cs_item_rev"))
   AND ("ss_item_rev" BETWEEN (DECIMAL '0.9' * "ws_item_rev") AND (DECIMAL '1.1' * "ws_item_rev"))
   AND ("cs_item_rev" BETWEEN (DECIMAL '0.9' * "ss_item_rev") AND (DECIMAL '1.1' * "ss_item_rev"))
   AND ("cs_item_rev" BETWEEN (DECIMAL '0.9' * "ws_item_rev") AND (DECIMAL '1.1' * "ws_item_rev"))
   AND ("ws_item_rev" BETWEEN (DECIMAL '0.9' * "ss_item_rev") AND (DECIMAL '1.1' * "ss_item_rev"))
   AND ("ws_item_rev" BETWEEN (DECIMAL '0.9' * "cs_item_rev") AND (DECIMAL '1.1' * "cs_item_rev"))
ORDER BY "ss_items"."item_id" ASC, "ss_item_rev" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q80.sql
WITH
  ssr AS (
   SELECT
     "s_store_id" "store_id"
   , "sum"("ss_ext_sales_price") "sales"
   , "sum"(COALESCE("sr_return_amt", 0)) "returns"
   , "sum"(("ss_net_profit" - COALESCE("sr_net_loss", 0))) "profit"
   FROM
     (catalog.schema.store_sales
   LEFT JOIN catalog.schema.store_returns ON ("ss_item_sk" = "sr_item_sk")
      AND ("ss_ticket_number" = "sr_ticket_number"))
   , catalog.schema.date_dim
   , catalog.schema.store
   , catalog.schema.item
   , catalog.schema.promotion
   WHERE ("ss_sold_date_sk" = "d_date_sk")
      AND (CAST("d_date" AS DATE) BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("ss_store_sk" = "s_store_sk")
      AND ("ss_item_sk" = "i_item_sk")
      AND ("i_current_price" > 50)
      AND ("ss_promo_sk" = "p_promo_sk")
      AND ("p_channel_tv" = 'N')
   GROUP BY "s_store_id"
) 
, csr AS (
   SELECT
     "cp_catalog_page_id" "catalog_page_id"
   , "sum"("cs_ext_sales_price") "sales"
   , "sum"(COALESCE("cr_return_amount", 0)) "returns"
   , "sum"(("cs_net_profit" - COALESCE("cr_net_loss", 0))) "profit"
   FROM
     (catalog.schema.catalog_sales
   LEFT JOIN catalog.schema.catalog_returns ON ("cs_item_sk" = "cr_item_sk")
      AND ("cs_order_number" = "cr_order_number"))
   , catalog.schema.date_dim
   , catalog.schema.catalog_page
   , catalog.schema.item
   , catalog.schema.promotion
   WHERE ("cs_sold_date_sk" = "d_date_sk")
      AND (CAST("d_date" AS DATE) BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("cs_catalog_page_sk" = "cp_catalog_page_sk")
      AND ("cs_item_sk" = "i_item_sk")
      AND ("i_current_price" > 50)
      AND ("cs_promo_sk" = "p_promo_sk")
      AND ("p_channel_tv" = 'N')
   GROUP BY "cp_catalog_page_id"
) 
, wsr AS (
   SELECT
     "web_site_id"
   , "sum"("ws_ext_sales_price") "sales"
   , "sum"(COALESCE("wr_return_amt", 0)) "returns"
   , "sum"(("ws_net_profit" - COALESCE("wr_net_loss", 0))) "profit"
   FROM
     (catalog.schema.web_sales
   LEFT JOIN catalog.schema.web_returns ON ("ws_item_sk" = "wr_item_sk")
      AND ("ws_order_number" = "wr_order_number"))
   , catalog.schema.date_dim
   , catalog.schema.web_site
   , catalog.schema.item
   , catalog.schema.promotion
   WHERE ("ws_sold_date_sk" = "d_date_sk")
      AND (CAST("d_date" AS DATE) BETWEEN CAST('2000-08-23' AS DATE) AND (CAST('2000-08-23' AS DATE) + INTERVAL  '30' DAY))
      AND ("ws_web_site_sk" = "web_site_sk")
      AND ("ws_item_sk" = "i_item_sk")
      AND ("i_current_price" > 50)
      AND ("ws_promo_sk" = "p_promo_sk")
      AND ("p_channel_tv" = 'N')
   GROUP BY "web_site_id"
) 
SELECT
  "channel"
, "id"
, "sum"("sales") "sales"
, "sum"("returns") "returns"
, "sum"("profit") "profit"
FROM
  (
   SELECT
     'store channel' "channel"
   , "concat"('store', "store_id") "id"
   , "sales"
   , "returns"
   , "profit"
   FROM
     ssr
UNION ALL    SELECT
     'catalog channel' "channel"
   , "concat"('catalog_page', "catalog_page_id") "id"
   , "sales"
   , "returns"
   , "profit"
   FROM
     csr
UNION ALL    SELECT
     'web channel' "channel"
   , "concat"('web_site', "web_site_id") "id"
   , "sales"
   , "returns"
   , "profit"
   FROM
     wsr
)  x
GROUP BY ROLLUP (channel, id)
ORDER BY "channel" ASC, "id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q09.sql
SELECT
  (CASE WHEN ((
      SELECT "count"(*)
      FROM
        catalog.schema.store_sales
      WHERE ("ss_quantity" BETWEEN 1 AND 20)
   ) > 74129) THEN (
   SELECT "avg"("ss_ext_discount_amt")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 1 AND 20)
) ELSE (
   SELECT "avg"("ss_net_paid")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 1 AND 20)
) END) "bucket1"
, (CASE WHEN ((
      SELECT "count"(*)
      FROM
        catalog.schema.store_sales
      WHERE ("ss_quantity" BETWEEN 21 AND 40)
   ) > 122840) THEN (
   SELECT "avg"("ss_ext_discount_amt")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 21 AND 40)
) ELSE (
   SELECT "avg"("ss_net_paid")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 21 AND 40)
) END) "bucket2"
, (CASE WHEN ((
      SELECT "count"(*)
      FROM
        catalog.schema.store_sales
      WHERE ("ss_quantity" BETWEEN 41 AND 60)
   ) > 56580) THEN (
   SELECT "avg"("ss_ext_discount_amt")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 41 AND 60)
) ELSE (
   SELECT "avg"("ss_net_paid")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 41 AND 60)
) END) "bucket3"
, (CASE WHEN ((
      SELECT "count"(*)
      FROM
        catalog.schema.store_sales
      WHERE ("ss_quantity" BETWEEN 61 AND 80)
   ) > 10097) THEN (
   SELECT "avg"("ss_ext_discount_amt")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 61 AND 80)
) ELSE (
   SELECT "avg"("ss_net_paid")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 61 AND 80)
) END) "bucket4"
, (CASE WHEN ((
      SELECT "count"(*)
      FROM
        catalog.schema.store_sales
      WHERE ("ss_quantity" BETWEEN 81 AND 100)
   ) > 165306) THEN (
   SELECT "avg"("ss_ext_discount_amt")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 81 AND 100)
) ELSE (
   SELECT "avg"("ss_net_paid")
   FROM
     catalog.schema.store_sales
   WHERE ("ss_quantity" BETWEEN 81 AND 100)
) END) "bucket5"
FROM
  catalog.schema.reason
WHERE ("r_reason_sk" = 1);

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q14.sql
WITH
  cross_items AS (
   SELECT "i_item_sk" "ss_item_sk"
   FROM
     catalog.schema.item
   , (
      SELECT
        "iss"."i_brand_id" "brand_id"
      , "iss"."i_class_id" "class_id"
      , "iss"."i_category_id" "category_id"
      FROM
        catalog.schema.store_sales
      , catalog.schema.item iss
      , catalog.schema.date_dim d1
      WHERE ("ss_item_sk" = "iss"."i_item_sk")
         AND ("ss_sold_date_sk" = "d1"."d_date_sk")
         AND ("d1"."d_year" BETWEEN 1999 AND (1999 + 2))
INTERSECT       SELECT
        "ics"."i_brand_id"
      , "ics"."i_class_id"
      , "ics"."i_category_id"
      FROM
        catalog.schema.catalog_sales
      , catalog.schema.item ics
      , catalog.schema.date_dim d2
      WHERE ("cs_item_sk" = "ics"."i_item_sk")
         AND ("cs_sold_date_sk" = "d2"."d_date_sk")
         AND ("d2"."d_year" BETWEEN 1999 AND (1999 + 2))
INTERSECT       SELECT
        "iws"."i_brand_id"
      , "iws"."i_class_id"
      , "iws"."i_category_id"
      FROM
        catalog.schema.web_sales
      , catalog.schema.item iws
      , catalog.schema.date_dim d3
      WHERE ("ws_item_sk" = "iws"."i_item_sk")
         AND ("ws_sold_date_sk" = "d3"."d_date_sk")
         AND ("d3"."d_year" BETWEEN 1999 AND (1999 + 2))
   ) 
   WHERE ("i_brand_id" = "brand_id")
      AND ("i_class_id" = "class_id")
      AND ("i_category_id" = "category_id")
) 
, avg_sales AS (
   SELECT "avg"(("quantity" * "list_price")) "average_sales"
   FROM
     (
      SELECT
        "ss_quantity" "quantity"
      , "ss_list_price" "list_price"
      FROM
        catalog.schema.store_sales
      , catalog.schema.date_dim
      WHERE ("ss_sold_date_sk" = "d_date_sk")
         AND ("d_year" BETWEEN 1999 AND (1999 + 2))
UNION ALL       SELECT
        "cs_quantity" "quantity"
      , "cs_list_price" "list_price"
      FROM
        catalog.schema.catalog_sales
      , catalog.schema.date_dim
      WHERE ("cs_sold_date_sk" = "d_date_sk")
         AND ("d_year" BETWEEN 1999 AND (1999 + 2))
UNION ALL       SELECT
        "ws_quantity" "quantity"
      , "ws_list_price" "list_price"
      FROM
        catalog.schema.web_sales
      , catalog.schema.date_dim
      WHERE ("ws_sold_date_sk" = "d_date_sk")
         AND ("d_year" BETWEEN 1999 AND (1999 + 2))
   )  x
) 
SELECT
  "channel"
, "i_brand_id"
, "i_class_id"
, "i_category_id"
, "sum"("sales")
, "sum"("number_sales")
FROM
  (
   SELECT
     'store' "channel"
   , "i_brand_id"
   , "i_class_id"
   , "i_category_id"
   , "sum"(("ss_quantity" * "ss_list_price")) "sales"
   , "count"(*) "number_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("ss_item_sk" IN (
      SELECT "ss_item_sk"
      FROM
        cross_items
   ))
      AND ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = (1999 + 2))
      AND ("d_moy" = 11)
   GROUP BY "i_brand_id", "i_class_id", "i_category_id"
   HAVING ("sum"(("ss_quantity" * "ss_list_price")) > (
         SELECT "average_sales"
         FROM
           avg_sales
      ))
UNION ALL    SELECT
     'catalog' "channel"
   , "i_brand_id"
   , "i_class_id"
   , "i_category_id"
   , "sum"(("cs_quantity" * "cs_list_price")) "sales"
   , "count"(*) "number_sales"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("cs_item_sk" IN (
      SELECT "ss_item_sk"
      FROM
        cross_items
   ))
      AND ("cs_item_sk" = "i_item_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_year" = (1999 + 2))
      AND ("d_moy" = 11)
   GROUP BY "i_brand_id", "i_class_id", "i_category_id"
   HAVING ("sum"(("cs_quantity" * "cs_list_price")) > (
         SELECT "average_sales"
         FROM
           avg_sales
      ))
UNION ALL    SELECT
     'web' "channel"
   , "i_brand_id"
   , "i_class_id"
   , "i_category_id"
   , "sum"(("ws_quantity" * "ws_list_price")) "sales"
   , "count"(*) "number_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("ws_item_sk" IN (
      SELECT "ss_item_sk"
      FROM
        cross_items
   ))
      AND ("ws_item_sk" = "i_item_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" = (1999 + 2))
      AND ("d_moy" = 11)
   GROUP BY "i_brand_id", "i_class_id", "i_category_id"
   HAVING ("sum"(("ws_quantity" * "ws_list_price")) > (
         SELECT "average_sales"
         FROM
           avg_sales
      ))
)  y
GROUP BY ROLLUP (channel, i_brand_id, i_class_id, i_category_id)
ORDER BY "channel" ASC, "i_brand_id" ASC, "i_class_id" ASC, "i_category_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q51.sql
WITH
  web_v1 AS (
   SELECT
     "ws_item_sk" "item_sk"
   , "d_date"
   , "sum"("sum"("ws_sales_price")) OVER (PARTITION BY "ws_item_sk" ORDER BY "d_date" ASC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) "cume_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   WHERE ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_month_seq" BETWEEN 1200 AND (1200 + 11))
      AND ("ws_item_sk" IS NOT NULL)
   GROUP BY "ws_item_sk", "d_date"
) 
, store_v1 AS (
   SELECT
     "ss_item_sk" "item_sk"
   , "d_date"
   , "sum"("sum"("ss_sales_price")) OVER (PARTITION BY "ss_item_sk" ORDER BY "d_date" ASC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) "cume_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_month_seq" BETWEEN 1200 AND (1200 + 11))
      AND ("ss_item_sk" IS NOT NULL)
   GROUP BY "ss_item_sk", "d_date"
) 
SELECT *
FROM
  (
   SELECT
     "item_sk"
   , "d_date"
   , "web_sales"
   , "store_sales"
   , "max"("web_sales") OVER (PARTITION BY "item_sk" ORDER BY "d_date" ASC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) "web_cumulative"
   , "max"("store_sales") OVER (PARTITION BY "item_sk" ORDER BY "d_date" ASC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) "store_cumulative"
   FROM
     (
      SELECT
        (CASE WHEN ("web"."item_sk" IS NOT NULL) THEN "web"."item_sk" ELSE "store"."item_sk" END) "item_sk"
      , (CASE WHEN ("web"."d_date" IS NOT NULL) THEN "web"."d_date" ELSE "store"."d_date" END) "d_date"
      , "web"."cume_sales" "web_sales"
      , "store"."cume_sales" "store_sales"
      FROM
        (web_v1 web
      FULL JOIN store_v1 store ON ("web"."item_sk" = "store"."item_sk")
         AND ("web"."d_date" = "store"."d_date"))
   )  x
)  y
WHERE ("web_cumulative" > "store_cumulative")
ORDER BY "item_sk" ASC, "d_date" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q21.sql
SELECT *
FROM
  (
   SELECT
     "w_warehouse_name"
   , "i_item_id"
   , "sum"((CASE WHEN (CAST("d_date" AS DATE) < CAST('2000-03-11' AS DATE)) THEN "inv_quantity_on_hand" ELSE 0 END)) "inv_before"
   , "sum"((CASE WHEN (CAST("d_date" AS DATE) >= CAST('2000-03-11' AS DATE)) THEN "inv_quantity_on_hand" ELSE 0 END)) "inv_after"
   FROM
     catalog.schema.inventory
   , catalog.schema.warehouse
   , catalog.schema.item
   , catalog.schema.date_dim
   WHERE ("i_current_price" BETWEEN DECIMAL '0.99' AND DECIMAL '1.49')
      AND ("i_item_sk" = "inv_item_sk")
      AND ("inv_warehouse_sk" = "w_warehouse_sk")
      AND ("inv_date_sk" = "d_date_sk")
      AND ("d_date" BETWEEN (CAST('2000-03-11' AS DATE) - INTERVAL  '30' DAY) AND (CAST('2000-03-11' AS DATE) + INTERVAL  '30' DAY))
   GROUP BY "w_warehouse_name", "i_item_id"
)  x
WHERE ((CASE WHEN ("inv_before" > 0) THEN (CAST("inv_after" AS DECIMAL(7,2)) / "inv_before") ELSE null END) BETWEEN (DECIMAL '2.00' / DECIMAL '3.00') AND (DECIMAL '3.00' / DECIMAL '2.00'))
ORDER BY "w_warehouse_name" ASC, "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q40.sql
SELECT
  "w_state"
, "i_item_id"
, "sum"((CASE WHEN (CAST("d_date" AS DATE) < CAST('2000-03-11' AS DATE)) THEN ("cs_sales_price" - COALESCE("cr_refunded_cash", 0)) ELSE 0 END)) "sales_before"
, "sum"((CASE WHEN (CAST("d_date" AS DATE) >= CAST('2000-03-11' AS DATE)) THEN ("cs_sales_price" - COALESCE("cr_refunded_cash", 0)) ELSE 0 END)) "sales_after"
FROM
  (catalog.schema.catalog_sales
LEFT JOIN catalog.schema.catalog_returns ON ("cs_order_number" = "cr_order_number")
   AND ("cs_item_sk" = "cr_item_sk"))
, catalog.schema.warehouse
, catalog.schema.item
, catalog.schema.date_dim
WHERE ("i_current_price" BETWEEN DECIMAL '0.99' AND DECIMAL '1.49')
   AND ("i_item_sk" = "cs_item_sk")
   AND ("cs_warehouse_sk" = "w_warehouse_sk")
   AND ("cs_sold_date_sk" = "d_date_sk")
   AND (CAST("d_date" AS DATE) BETWEEN (CAST('2000-03-11' AS DATE) - INTERVAL  '30' DAY) AND (CAST('2000-03-11' AS DATE) + INTERVAL  '30' DAY))
GROUP BY "w_state", "i_item_id"
ORDER BY "w_state" ASC, "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q23.sql
WITH
  frequent_ss_items AS (
   SELECT
     "substr"("i_item_desc", 1, 30) "itemdesc"
   , "i_item_sk" "item_sk"
   , "d_date" "solddate"
   , "count"(*) "cnt"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.item
   WHERE ("ss_sold_date_sk" = "d_date_sk")
      AND ("ss_item_sk" = "i_item_sk")
      AND ("d_year" IN (2000   , (2000 + 1)   , (2000 + 2)   , (2000 + 3)))
   GROUP BY "substr"("i_item_desc", 1, 30), "i_item_sk", "d_date"
   HAVING ("count"(*) > 4)
) 
, max_store_sales AS (
   SELECT "max"("csales") "tpcds_cmax"
   FROM
     (
      SELECT
        "c_customer_sk"
      , "sum"(("ss_quantity" * "ss_sales_price")) "csales"
      FROM
        catalog.schema.store_sales
      , catalog.schema.customer
      , catalog.schema.date_dim
      WHERE ("ss_customer_sk" = "c_customer_sk")
         AND ("ss_sold_date_sk" = "d_date_sk")
         AND ("d_year" IN (2000      , (2000 + 1)      , (2000 + 2)      , (2000 + 3)))
      GROUP BY "c_customer_sk"
   ) 
) 
, best_ss_customer AS (
   SELECT
     "c_customer_sk"
   , "sum"(("ss_quantity" * "ss_sales_price")) "ssales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.customer
   WHERE ("ss_customer_sk" = "c_customer_sk")
   GROUP BY "c_customer_sk"
   HAVING ("sum"(("ss_quantity" * "ss_sales_price")) > ((50 / DECIMAL '100.0') * (
            SELECT *
            FROM
              max_store_sales
         )))
) 
SELECT "sum"("sales")
FROM
  (
   SELECT ("cs_quantity" * "cs_list_price") "sales"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   WHERE ("d_year" = 2000)
      AND ("d_moy" = 2)
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("cs_item_sk" IN (
      SELECT "item_sk"
      FROM
        frequent_ss_items
   ))
      AND ("cs_bill_customer_sk" IN (
      SELECT "c_customer_sk"
      FROM
        best_ss_customer
   ))
UNION ALL    SELECT ("ws_quantity" * "ws_list_price") "sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   WHERE ("d_year" = 2000)
      AND ("d_moy" = 2)
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("ws_item_sk" IN (
      SELECT "item_sk"
      FROM
        frequent_ss_items
   ))
      AND ("ws_bill_customer_sk" IN (
      SELECT "c_customer_sk"
      FROM
        best_ss_customer
   ))
) 
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q56.sql
WITH
  ss AS (
   SELECT
     "i_item_id"
   , "sum"("ss_ext_sales_price") "total_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_color" IN ('slate'      , 'blanched'      , 'burnished'))
   ))
      AND ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" = 2)
      AND ("ss_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
, cs AS (
   SELECT
     "i_item_id"
   , "sum"("cs_ext_sales_price") "total_sales"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_color" IN ('slate'      , 'blanched'      , 'burnished'))
   ))
      AND ("cs_item_sk" = "i_item_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" = 2)
      AND ("cs_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
, ws AS (
   SELECT
     "i_item_id"
   , "sum"("ws_ext_sales_price") "total_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_color" IN ('slate'      , 'blanched'      , 'burnished'))
   ))
      AND ("ws_item_sk" = "i_item_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" = 2)
      AND ("ws_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
SELECT
  "i_item_id"
, "sum"("total_sales") "total_sales"
FROM
  (
   SELECT *
   FROM
     ss
UNION ALL    SELECT *
   FROM
     cs
UNION ALL    SELECT *
   FROM
     ws
)  tmp1
GROUP BY "i_item_id"
ORDER BY "total_sales" ASC, "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q33.sql
WITH
  ss AS (
   SELECT
     "i_manufact_id"
   , "sum"("ss_ext_sales_price") "total_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_manufact_id" IN (
      SELECT "i_manufact_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Electronics'))
   ))
      AND ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 5)
      AND ("ss_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_manufact_id"
) 
, cs AS (
   SELECT
     "i_manufact_id"
   , "sum"("cs_ext_sales_price") "total_sales"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_manufact_id" IN (
      SELECT "i_manufact_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Electronics'))
   ))
      AND ("cs_item_sk" = "i_item_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 5)
      AND ("cs_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_manufact_id"
) 
, ws AS (
   SELECT
     "i_manufact_id"
   , "sum"("ws_ext_sales_price") "total_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_manufact_id" IN (
      SELECT "i_manufact_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Electronics'))
   ))
      AND ("ws_item_sk" = "i_item_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 5)
      AND ("ws_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_manufact_id"
) 
SELECT
  "i_manufact_id"
, "sum"("total_sales") "total_sales"
FROM
  (
   SELECT *
   FROM
     ss
UNION ALL    SELECT *
   FROM
     cs
UNION ALL    SELECT *
   FROM
     ws
)  tmp1
GROUP BY "i_manufact_id"
ORDER BY "total_sales" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q60.sql
WITH
  ss AS (
   SELECT
     "i_item_id"
   , "sum"("ss_ext_sales_price") "total_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Music'))
   ))
      AND ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 9)
      AND ("ss_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
, cs AS (
   SELECT
     "i_item_id"
   , "sum"("cs_ext_sales_price") "total_sales"
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Music'))
   ))
      AND ("cs_item_sk" = "i_item_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 9)
      AND ("cs_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
, ws AS (
   SELECT
     "i_item_id"
   , "sum"("ws_ext_sales_price") "total_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   , catalog.schema.item
   WHERE ("i_item_id" IN (
      SELECT "i_item_id"
      FROM
        catalog.schema.item
      WHERE ("i_category" IN ('Music'))
   ))
      AND ("ws_item_sk" = "i_item_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 1998)
      AND ("d_moy" = 9)
      AND ("ws_bill_addr_sk" = "ca_address_sk")
      AND ("ca_gmt_offset" = -5)
   GROUP BY "i_item_id"
) 
SELECT
  "i_item_id"
, "sum"("total_sales") "total_sales"
FROM
  (
   SELECT *
   FROM
     ss
UNION ALL    SELECT *
   FROM
     cs
UNION ALL    SELECT *
   FROM
     ws
)  tmp1
GROUP BY "i_item_id"
ORDER BY "i_item_id" ASC, "total_sales" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q53.sql
SELECT *
FROM
  (
   SELECT
     "i_manufact_id"
   , "sum"("ss_sales_price") "sum_sales"
   , "avg"("sum"("ss_sales_price")) OVER (PARTITION BY "i_manufact_id") "avg_quarterly_sales"
   FROM
     catalog.schema.item
   , catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("ss_store_sk" = "s_store_sk")
      AND ("d_month_seq" IN (1200   , (1200 + 1)   , (1200 + 2)   , (1200 + 3)   , (1200 + 4)   , (1200 + 5)   , (1200 + 6)   , (1200 + 7)   , (1200 + 8)   , (1200 + 9)   , (1200 + 10)   , (1200 + 11)))
      AND ((("i_category" IN ('Books'         , 'Children'         , 'Electronics'))
            AND ("i_class" IN ('personal'         , 'portable'         , 'reference'         , 'self-help'))
            AND ("i_brand" IN ('scholaramalgamalg #14'         , 'scholaramalgamalg #7'         , 'exportiunivamalg #9'         , 'scholaramalgamalg #9')))
         OR (("i_category" IN ('Women'         , 'Music'         , 'Men'))
            AND ("i_class" IN ('accessories'         , 'classical'         , 'fragrances'         , 'pants'))
            AND ("i_brand" IN ('amalgimporto #1'         , 'edu packscholar #1'         , 'exportiimporto #1'         , 'importoamalg #1'))))
   GROUP BY "i_manufact_id", "d_qoy"
)  tmp1
WHERE ((CASE WHEN ("avg_quarterly_sales" > 0) THEN ("abs"((CAST("sum_sales" AS DECIMAL(38,4)) - "avg_quarterly_sales")) / "avg_quarterly_sales") ELSE null END) > DECIMAL '0.1')
ORDER BY "avg_quarterly_sales" ASC, "sum_sales" ASC, "i_manufact_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q59.sql
WITH
  wss AS (
   SELECT
     "d_week_seq"
   , "ss_store_sk"
   , "sum"((CASE WHEN ("d_day_name" = 'Sunday') THEN "ss_sales_price" ELSE null END)) "sun_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Monday') THEN "ss_sales_price" ELSE null END)) "mon_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Tuesday') THEN "ss_sales_price" ELSE null END)) "tue_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Wednesday') THEN "ss_sales_price" ELSE null END)) "wed_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Thursday') THEN "ss_sales_price" ELSE null END)) "thu_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Friday') THEN "ss_sales_price" ELSE null END)) "fri_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Saturday') THEN "ss_sales_price" ELSE null END)) "sat_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("d_date_sk" = "ss_sold_date_sk")
   GROUP BY "d_week_seq", "ss_store_sk"
) 
SELECT
  "s_store_name1"
, "s_store_id1"
, "d_week_seq1"
, ("sun_sales1" / "sun_sales2")
, ("mon_sales1" / "mon_sales2")
, ("tue_sales1" / "tue_sales2")
, ("wed_sales1" / "wed_sales2")
, ("thu_sales1" / "thu_sales2")
, ("fri_sales1" / "fri_sales2")
, ("sat_sales1" / "sat_sales2")
FROM
  (
   SELECT
     "s_store_name" "s_store_name1"
   , "wss"."d_week_seq" "d_week_seq1"
   , "s_store_id" "s_store_id1"
   , "sun_sales" "sun_sales1"
   , "mon_sales" "mon_sales1"
   , "tue_sales" "tue_sales1"
   , "wed_sales" "wed_sales1"
   , "thu_sales" "thu_sales1"
   , "fri_sales" "fri_sales1"
   , "sat_sales" "sat_sales1"
   FROM
     wss
   , catalog.schema.store
   , catalog.schema.date_dim d
   WHERE ("d"."d_week_seq" = "wss"."d_week_seq")
      AND ("ss_store_sk" = "s_store_sk")
      AND ("d_month_seq" BETWEEN 1212 AND (1212 + 11))
)  y
, (
   SELECT
     "s_store_name" "s_store_name2"
   , "wss"."d_week_seq" "d_week_seq2"
   , "s_store_id" "s_store_id2"
   , "sun_sales" "sun_sales2"
   , "mon_sales" "mon_sales2"
   , "tue_sales" "tue_sales2"
   , "wed_sales" "wed_sales2"
   , "thu_sales" "thu_sales2"
   , "fri_sales" "fri_sales2"
   , "sat_sales" "sat_sales2"
   FROM
     wss
   , catalog.schema.store
   , catalog.schema.date_dim d
   WHERE ("d"."d_week_seq" = "wss"."d_week_seq")
      AND ("ss_store_sk" = "s_store_sk")
      AND ("d_month_seq" BETWEEN (1212 + 12) AND (1212 + 23))
)  x
WHERE ("s_store_id1" = "s_store_id2")
   AND ("d_week_seq1" = ("d_week_seq2" - 52))
ORDER BY "s_store_name1" ASC, "s_store_id1" ASC, "d_week_seq1" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q31.sql
WITH
  ss AS (
   SELECT
     "ca_county"
   , "d_qoy"
   , "d_year"
   , "sum"("ss_ext_sales_price") "store_sales"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   WHERE ("ss_sold_date_sk" = "d_date_sk")
      AND ("ss_addr_sk" = "ca_address_sk")
   GROUP BY "ca_county", "d_qoy", "d_year"
) 
, ws AS (
   SELECT
     "ca_county"
   , "d_qoy"
   , "d_year"
   , "sum"("ws_ext_sales_price") "web_sales"
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   , catalog.schema.customer_address
   WHERE ("ws_sold_date_sk" = "d_date_sk")
      AND ("ws_bill_addr_sk" = "ca_address_sk")
   GROUP BY "ca_county", "d_qoy", "d_year"
) 
SELECT
  "ss1"."ca_county"
, "ss1"."d_year"
, ("ws2"."web_sales" / "ws1"."web_sales") "web_q1_q2_increase"
, ("ss2"."store_sales" / "ss1"."store_sales") "store_q1_q2_increase"
, ("ws3"."web_sales" / "ws2"."web_sales") "web_q2_q3_increase"
, ("ss3"."store_sales" / "ss2"."store_sales") "store_q2_q3_increase"
FROM
  ss ss1
, ss ss2
, ss ss3
, ws ws1
, ws ws2
, ws ws3
WHERE ("ss1"."d_qoy" = 1)
   AND ("ss1"."d_year" = 2000)
   AND ("ss1"."ca_county" = "ss2"."ca_county")
   AND ("ss2"."d_qoy" = 2)
   AND ("ss2"."d_year" = 2000)
   AND ("ss2"."ca_county" = "ss3"."ca_county")
   AND ("ss3"."d_qoy" = 3)
   AND ("ss3"."d_year" = 2000)
   AND ("ss1"."ca_county" = "ws1"."ca_county")
   AND ("ws1"."d_qoy" = 1)
   AND ("ws1"."d_year" = 2000)
   AND ("ws1"."ca_county" = "ws2"."ca_county")
   AND ("ws2"."d_qoy" = 2)
   AND ("ws2"."d_year" = 2000)
   AND ("ws1"."ca_county" = "ws3"."ca_county")
   AND ("ws3"."d_qoy" = 3)
   AND ("ws3"."d_year" = 2000)
   AND ((CASE WHEN ("ws1"."web_sales" > 0) THEN (CAST("ws2"."web_sales" AS DECIMAL(38,3)) / "ws1"."web_sales") ELSE null END) > (CASE WHEN ("ss1"."store_sales" > 0) THEN (CAST("ss2"."store_sales" AS DECIMAL(38,3)) / "ss1"."store_sales") ELSE null END))
   AND ((CASE WHEN ("ws2"."web_sales" > 0) THEN (CAST("ws3"."web_sales" AS DECIMAL(38,3)) / "ws2"."web_sales") ELSE null END) > (CASE WHEN ("ss2"."store_sales" > 0) THEN (CAST("ss3"."store_sales" AS DECIMAL(38,3)) / "ss2"."store_sales") ELSE null END))
ORDER BY "ss1"."ca_county" ASC;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q63.sql
SELECT *
FROM
  (
   SELECT
     "i_manager_id"
   , "sum"("ss_sales_price") "sum_sales"
   , "avg"("sum"("ss_sales_price")) OVER (PARTITION BY "i_manager_id") "avg_monthly_sales"
   FROM
     catalog.schema.item
   , catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("ss_store_sk" = "s_store_sk")
      AND ("d_month_seq" IN (1200   , (1200 + 1)   , (1200 + 2)   , (1200 + 3)   , (1200 + 4)   , (1200 + 5)   , (1200 + 6)   , (1200 + 7)   , (1200 + 8)   , (1200 + 9)   , (1200 + 10)   , (1200 + 11)))
      AND ((("i_category" IN ('Books'         , 'Children'         , 'Electronics'))
            AND ("i_class" IN ('personal'         , 'portable'         , 'refernece'         , 'self-help'))
            AND ("i_brand" IN ('scholaramalgamalg #14'         , 'scholaramalgamalg #7'         , 'exportiunivamalg #9'         , 'scholaramalgamalg #9')))
         OR (("i_category" IN ('Women'         , 'Music'         , 'Men'))
            AND ("i_class" IN ('accessories'         , 'classical'         , 'fragrances'         , 'pants'))
            AND ("i_brand" IN ('amalgimporto #1'         , 'edu packscholar #1'         , 'exportiimporto #1'         , 'importoamalg #1'))))
   GROUP BY "i_manager_id", "d_moy"
)  tmp1
WHERE ((CASE WHEN ("avg_monthly_sales" > 0) THEN ("abs"(("sum_sales" - "avg_monthly_sales")) / "avg_monthly_sales") ELSE null END) > DECIMAL '0.1')
ORDER BY "i_manager_id" ASC, "avg_monthly_sales" ASC, "sum_sales" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q70.sql
SELECT
  "sum"("ss_net_profit") "total_sum"
, "s_state"
, "s_county"
, (GROUPING ("s_state") + GROUPING ("s_county")) "lochierarchy"
, "rank"() OVER (PARTITION BY (GROUPING ("s_state") + GROUPING ("s_county")), (CASE WHEN (GROUPING ("s_county") = 0) THEN "s_state" END) ORDER BY "sum"("ss_net_profit") DESC) "rank_within_parent"
FROM
  catalog.schema.store_sales
, catalog.schema.date_dim d1
, catalog.schema.store
WHERE ("d1"."d_month_seq" BETWEEN 1200 AND (1200 + 11))
   AND ("d1"."d_date_sk" = "ss_sold_date_sk")
   AND ("s_store_sk" = "ss_store_sk")
   AND ("s_state" IN (
   SELECT "s_state"
   FROM
     (
      SELECT
        "s_state" "s_state"
      , "rank"() OVER (PARTITION BY "s_state" ORDER BY "sum"("ss_net_profit") DESC) "ranking"
      FROM
        catalog.schema.store_sales
      , catalog.schema.store
      , catalog.schema.date_dim
      WHERE ("d_month_seq" BETWEEN 1200 AND (1200 + 11))
         AND ("d_date_sk" = "ss_sold_date_sk")
         AND ("s_store_sk" = "ss_store_sk")
      GROUP BY "s_state"
   )  tmp1
   WHERE ("ranking" <= 5)
))
GROUP BY ROLLUP (s_state, s_county)
ORDER BY "lochierarchy" DESC, (CASE WHEN ("lochierarchy" = 0) THEN "s_state" END) ASC, "rank_within_parent" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q04.sql
WITH
  year_total AS (
   SELECT
     "c_customer_id" "customer_id"
   , "c_first_name" "customer_first_name"
   , "c_last_name" "customer_last_name"
   , "c_preferred_cust_flag" "customer_preferred_cust_flag"
   , "c_birth_country" "customer_birth_country"
   , "c_login" "customer_login"
   , "c_email_address" "customer_email_address"
   , "d_year" "dyear"
   , "sum"((((("ss_ext_list_price" - "ss_ext_wholesale_cost") - "ss_ext_discount_amt") + "ss_ext_sales_price") / 2)) "year_total"
   , 's' "sale_type"
   FROM
     catalog.schema.customer
   , catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("c_customer_sk" = "ss_customer_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
   GROUP BY "c_customer_id", "c_first_name", "c_last_name", "c_preferred_cust_flag", "c_birth_country", "c_login", "c_email_address", "d_year"
UNION ALL    SELECT
     "c_customer_id" "customer_id"
   , "c_first_name" "customer_first_name"
   , "c_last_name" "customer_last_name"
   , "c_preferred_cust_flag" "customer_preferred_cust_flag"
   , "c_birth_country" "customer_birth_country"
   , "c_login" "customer_login"
   , "c_email_address" "customer_email_address"
   , "d_year" "dyear"
   , "sum"((((("cs_ext_list_price" - "cs_ext_wholesale_cost") - "cs_ext_discount_amt") + "cs_ext_sales_price") / 2)) "year_total"
   , 'c' "sale_type"
   FROM
     catalog.schema.customer
   , catalog.schema.catalog_sales
   , catalog.schema.date_dim
   WHERE ("c_customer_sk" = "cs_bill_customer_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
   GROUP BY "c_customer_id", "c_first_name", "c_last_name", "c_preferred_cust_flag", "c_birth_country", "c_login", "c_email_address", "d_year"
UNION ALL    SELECT
     "c_customer_id" "customer_id"
   , "c_first_name" "customer_first_name"
   , "c_last_name" "customer_last_name"
   , "c_preferred_cust_flag" "customer_preferred_cust_flag"
   , "c_birth_country" "customer_birth_country"
   , "c_login" "customer_login"
   , "c_email_address" "customer_email_address"
   , "d_year" "dyear"
   , "sum"((((("ws_ext_list_price" - "ws_ext_wholesale_cost") - "ws_ext_discount_amt") + "ws_ext_sales_price") / 2)) "year_total"
   , 'w' "sale_type"
   FROM
     catalog.schema.customer
   , catalog.schema.web_sales
   , catalog.schema.date_dim
   WHERE ("c_customer_sk" = "ws_bill_customer_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
   GROUP BY "c_customer_id", "c_first_name", "c_last_name", "c_preferred_cust_flag", "c_birth_country", "c_login", "c_email_address", "d_year"
) 
SELECT
  "t_s_secyear"."customer_id"
, "t_s_secyear"."customer_first_name"
, "t_s_secyear"."customer_last_name"
, "t_s_secyear"."customer_preferred_cust_flag"
FROM
  year_total t_s_firstyear
, year_total t_s_secyear
, year_total t_c_firstyear
, year_total t_c_secyear
, year_total t_w_firstyear
, year_total t_w_secyear
WHERE ("t_s_secyear"."customer_id" = "t_s_firstyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_c_secyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_c_firstyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_w_firstyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_w_secyear"."customer_id")
   AND ("t_s_firstyear"."sale_type" = 's')
   AND ("t_c_firstyear"."sale_type" = 'c')
   AND ("t_w_firstyear"."sale_type" = 'w')
   AND ("t_s_secyear"."sale_type" = 's')
   AND ("t_c_secyear"."sale_type" = 'c')
   AND ("t_w_secyear"."sale_type" = 'w')
   AND ("t_s_firstyear"."dyear" = 2001)
   AND ("t_s_secyear"."dyear" = (2001 + 1))
   AND ("t_c_firstyear"."dyear" = 2001)
   AND ("t_c_secyear"."dyear" = (2001 + 1))
   AND ("t_w_firstyear"."dyear" = 2001)
   AND ("t_w_secyear"."dyear" = (2001 + 1))
   AND ("t_s_firstyear"."year_total" > 0)
   AND ("t_c_firstyear"."year_total" > 0)
   AND ("t_w_firstyear"."year_total" > 0)
   AND ((CASE WHEN ("t_c_firstyear"."year_total" > 0) THEN ("t_c_secyear"."year_total" / "t_c_firstyear"."year_total") ELSE null END) > (CASE WHEN ("t_s_firstyear"."year_total" > 0) THEN ("t_s_secyear"."year_total" / "t_s_firstyear"."year_total") ELSE null END))
   AND ((CASE WHEN ("t_c_firstyear"."year_total" > 0) THEN ("t_c_secyear"."year_total" / "t_c_firstyear"."year_total") ELSE null END) > (CASE WHEN ("t_w_firstyear"."year_total" > 0) THEN ("t_w_secyear"."year_total" / "t_w_firstyear"."year_total") ELSE null END))
ORDER BY "t_s_secyear"."customer_id" ASC, "t_s_secyear"."customer_first_name" ASC, "t_s_secyear"."customer_last_name" ASC, "t_s_secyear"."customer_preferred_cust_flag" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q02.sql
WITH
  wscs AS (
   SELECT
     "sold_date_sk"
   , "sales_price"
   FROM
     (
      SELECT
        "ws_sold_date_sk" "sold_date_sk"
      , "ws_ext_sales_price" "sales_price"
      FROM
        catalog.schema.web_sales
   )  
UNION ALL (
      SELECT
        "cs_sold_date_sk" "sold_date_sk"
      , "cs_ext_sales_price" "sales_price"
      FROM
        catalog.schema.catalog_sales
   ) ) 
, wswscs AS (
   SELECT
     "d_week_seq"
   , "sum"((CASE WHEN ("d_day_name" = 'Sunday') THEN "sales_price" ELSE null END)) "sun_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Monday') THEN "sales_price" ELSE null END)) "mon_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Tuesday') THEN "sales_price" ELSE null END)) "tue_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Wednesday') THEN "sales_price" ELSE null END)) "wed_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Thursday') THEN "sales_price" ELSE null END)) "thu_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Friday') THEN "sales_price" ELSE null END)) "fri_sales"
   , "sum"((CASE WHEN ("d_day_name" = 'Saturday') THEN "sales_price" ELSE null END)) "sat_sales"
   FROM
     wscs
   , catalog.schema.date_dim
   WHERE ("d_date_sk" = "sold_date_sk")
   GROUP BY "d_week_seq"
) 
SELECT
  "d_week_seq1"
, "round"(("sun_sales1" / "sun_sales2"), 2)
, "round"(("mon_sales1" / "mon_sales2"), 2)
, "round"(("tue_sales1" / "tue_sales2"), 2)
, "round"(("wed_sales1" / "wed_sales2"), 2)
, "round"(("thu_sales1" / "thu_sales2"), 2)
, "round"(("fri_sales1" / "fri_sales2"), 2)
, "round"(("sat_sales1" / "sat_sales2"), 2)
FROM
  (
   SELECT
     "wswscs"."d_week_seq" "d_week_seq1"
   , "sun_sales" "sun_sales1"
   , "mon_sales" "mon_sales1"
   , "tue_sales" "tue_sales1"
   , "wed_sales" "wed_sales1"
   , "thu_sales" "thu_sales1"
   , "fri_sales" "fri_sales1"
   , "sat_sales" "sat_sales1"
   FROM
     wswscs
   , catalog.schema.date_dim
   WHERE ("date_dim"."d_week_seq" = "wswscs"."d_week_seq")
      AND ("d_year" = 2001)
)  y
, (
   SELECT
     "wswscs"."d_week_seq" "d_week_seq2"
   , "sun_sales" "sun_sales2"
   , "mon_sales" "mon_sales2"
   , "tue_sales" "tue_sales2"
   , "wed_sales" "wed_sales2"
   , "thu_sales" "thu_sales2"
   , "fri_sales" "fri_sales2"
   , "sat_sales" "sat_sales2"
   FROM
     wswscs
   , catalog.schema.date_dim
   WHERE ("date_dim"."d_week_seq" = "wswscs"."d_week_seq")
      AND ("d_year" = (2001 + 1))
)  z
WHERE ("d_week_seq1" = ("d_week_seq2" - 53))
ORDER BY "d_week_seq1" ASC;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q85.sql
SELECT
  "substr"("r_reason_desc", 1, 20)
, "avg"("ws_quantity")
, "avg"("wr_refunded_cash")
, "avg"("wr_fee")
FROM
  catalog.schema.web_sales
, catalog.schema.web_returns
, catalog.schema.web_page
, catalog.schema.customer_demographics cd1
, catalog.schema.customer_demographics cd2
, catalog.schema.customer_address
, catalog.schema.date_dim
, catalog.schema.reason
WHERE ("ws_web_page_sk" = "wp_web_page_sk")
   AND ("ws_item_sk" = "wr_item_sk")
   AND ("ws_order_number" = "wr_order_number")
   AND ("ws_sold_date_sk" = "d_date_sk")
   AND ("d_year" = 2000)
   AND ("cd1"."cd_demo_sk" = "wr_refunded_cdemo_sk")
   AND ("cd2"."cd_demo_sk" = "wr_returning_cdemo_sk")
   AND ("ca_address_sk" = "wr_refunded_addr_sk")
   AND ("r_reason_sk" = "wr_reason_sk")
   AND ((("cd1"."cd_marital_status" = 'M')
         AND ("cd1"."cd_marital_status" = "cd2"."cd_marital_status")
         AND ("cd1"."cd_education_status" = 'Advanced Degree')
         AND ("cd1"."cd_education_status" = "cd2"."cd_education_status")
         AND ("ws_sales_price" BETWEEN DECIMAL '100.00' AND DECIMAL '150.00'))
      OR (("cd1"."cd_marital_status" = 'S')
         AND ("cd1"."cd_marital_status" = "cd2"."cd_marital_status")
         AND ("cd1"."cd_education_status" = 'College')
         AND ("cd1"."cd_education_status" = "cd2"."cd_education_status")
         AND ("ws_sales_price" BETWEEN DECIMAL '50.00' AND DECIMAL '100.00'))
      OR (("cd1"."cd_marital_status" = 'W')
         AND ("cd1"."cd_marital_status" = "cd2"."cd_marital_status")
         AND ("cd1"."cd_education_status" = '2 yr Degree')
         AND ("cd1"."cd_education_status" = "cd2"."cd_education_status")
         AND ("ws_sales_price" BETWEEN DECIMAL '150.00' AND DECIMAL '200.00')))
   AND ((("ca_country" = 'United States')
         AND ("ca_state" IN ('IN'      , 'OH'      , 'NJ'))
         AND ("ws_net_profit" BETWEEN 100 AND 200))
      OR (("ca_country" = 'United States')
         AND ("ca_state" IN ('WI'      , 'CT'      , 'KY'))
         AND ("ws_net_profit" BETWEEN 150 AND 300))
      OR (("ca_country" = 'United States')
         AND ("ca_state" IN ('LA'      , 'IA'      , 'AR'))
         AND ("ws_net_profit" BETWEEN 50 AND 250)))
GROUP BY "r_reason_desc"
ORDER BY "substr"("r_reason_desc", 1, 20) ASC, "avg"("ws_quantity") ASC, "avg"("wr_refunded_cash") ASC, "avg"("wr_fee") ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q13.sql
SELECT
  "avg"("ss_quantity")
, "avg"("ss_ext_sales_price")
, "avg"("ss_ext_wholesale_cost")
, "sum"("ss_ext_wholesale_cost")
FROM
  catalog.schema.store_sales
, catalog.schema.store
, catalog.schema.customer_demographics
, catalog.schema.household_demographics
, catalog.schema.customer_address
, catalog.schema.date_dim
WHERE ("s_store_sk" = "ss_store_sk")
   AND ("ss_sold_date_sk" = "d_date_sk")
   AND ("d_year" = 2001)
   AND ((("ss_hdemo_sk" = "hd_demo_sk")
         AND ("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'M')
         AND ("cd_education_status" = 'Advanced Degree')
         AND ("ss_sales_price" BETWEEN DECIMAL '100.00' AND DECIMAL '150.00')
         AND ("hd_dep_count" = 3))
      OR (("ss_hdemo_sk" = "hd_demo_sk")
         AND ("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'S')
         AND ("cd_education_status" = 'College')
         AND ("ss_sales_price" BETWEEN DECIMAL '50.00' AND DECIMAL '100.00')
         AND ("hd_dep_count" = 1))
      OR (("ss_hdemo_sk" = "hd_demo_sk")
         AND ("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'W')
         AND ("cd_education_status" = '2 yr Degree')
         AND ("ss_sales_price" BETWEEN DECIMAL '150.00' AND DECIMAL '200.00')
         AND ("hd_dep_count" = 1)))
   AND ((("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('TX'      , 'OH'      , 'TX'))
         AND ("ss_net_profit" BETWEEN 100 AND 200))
      OR (("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('OR'      , 'NM'      , 'KY'))
         AND ("ss_net_profit" BETWEEN 150 AND 300))
      OR (("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('VA'      , 'TX'      , 'MS'))
         AND ("ss_net_profit" BETWEEN 50 AND 250)));

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q48.sql
SELECT "sum"("ss_quantity")
FROM
  catalog.schema.store_sales
, catalog.schema.store
, catalog.schema.customer_demographics
, catalog.schema.customer_address
, catalog.schema.date_dim
WHERE ("s_store_sk" = "ss_store_sk")
   AND ("ss_sold_date_sk" = "d_date_sk")
   AND ("d_year" = 2000)
   AND ((("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'M')
         AND ("cd_education_status" = '4 yr Degree')
         AND ("ss_sales_price" BETWEEN DECIMAL '100.00' AND DECIMAL '150.00'))
      OR (("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'D')
         AND ("cd_education_status" = '2 yr Degree')
         AND ("ss_sales_price" BETWEEN DECIMAL '50.00' AND DECIMAL '100.00'))
      OR (("cd_demo_sk" = "ss_cdemo_sk")
         AND ("cd_marital_status" = 'S')
         AND ("cd_education_status" = 'College')
         AND ("ss_sales_price" BETWEEN DECIMAL '150.00' AND DECIMAL '200.00')))
   AND ((("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('CO'      , 'OH'      , 'TX'))
         AND ("ss_net_profit" BETWEEN 0 AND 2000))
      OR (("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('OR'      , 'MN'      , 'KY'))
         AND ("ss_net_profit" BETWEEN 150 AND 3000))
      OR (("ss_addr_sk" = "ca_address_sk")
         AND ("ca_country" = 'United States')
         AND ("ca_state" IN ('VA'      , 'CA'      , 'MS'))
         AND ("ss_net_profit" BETWEEN 50 AND 25000)));

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q18.sql
SELECT
  "i_item_id"
, "ca_country"
, "ca_state"
, "ca_county"
, "avg"(CAST("cs_quantity" AS DECIMAL(12,2))) "agg1"
, "avg"(CAST("cs_list_price" AS DECIMAL(12,2))) "agg2"
, "avg"(CAST("cs_coupon_amt" AS DECIMAL(12,2))) "agg3"
, "avg"(CAST("cs_sales_price" AS DECIMAL(12,2))) "agg4"
, "avg"(CAST("cs_net_profit" AS DECIMAL(12,2))) "agg5"
, "avg"(CAST("c_birth_year" AS DECIMAL(12,2))) "agg6"
, "avg"(CAST("cd1"."cd_dep_count" AS DECIMAL(12,2))) "agg7"
FROM
  catalog.schema.catalog_sales
, catalog.schema.customer_demographics cd1
, catalog.schema.customer_demographics cd2
, catalog.schema.customer
, catalog.schema.customer_address
, catalog.schema.date_dim
, catalog.schema.item
WHERE ("cs_sold_date_sk" = "d_date_sk")
   AND ("cs_item_sk" = "i_item_sk")
   AND ("cs_bill_cdemo_sk" = "cd1"."cd_demo_sk")
   AND ("cs_bill_customer_sk" = "c_customer_sk")
   AND ("cd1"."cd_gender" = 'F')
   AND ("cd1"."cd_education_status" = 'Unknown')
   AND ("c_current_cdemo_sk" = "cd2"."cd_demo_sk")
   AND ("c_current_addr_sk" = "ca_address_sk")
   AND ("c_birth_month" IN (1, 6, 8, 9, 12, 2))
   AND ("d_year" = 1998)
   AND ("ca_state" IN ('MS', 'IN', 'ND', 'OK', 'NM', 'VA', 'MS'))
GROUP BY ROLLUP (i_item_id, ca_country, ca_state, ca_county)
ORDER BY "ca_country" ASC, "ca_state" ASC, "ca_county" ASC, "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q44.sql
SELECT
  "asceding"."rnk"
, "i1"."i_product_name" "best_performing"
, "i2"."i_product_name" "worst_performing"
FROM
  (
   SELECT *
   FROM
     (
      SELECT
        "item_sk"
      , "rank"() OVER (ORDER BY "rank_col" ASC) "rnk"
      FROM
        (
         SELECT
           "ss_item_sk" "item_sk"
         , "avg"("ss_net_profit") "rank_col"
         FROM
           catalog.schema.store_sales ss1
         WHERE ("ss_store_sk" = 4)
         GROUP BY "ss_item_sk"
         HAVING ("avg"("ss_net_profit") > (DECIMAL '0.9' * (
                  SELECT "avg"("ss_net_profit") "rank_col"
                  FROM
                    catalog.schema.store_sales
                  WHERE ("ss_store_sk" = 4)
                     AND ("ss_addr_sk" IS NULL)
                  GROUP BY "ss_store_sk"
               )))
      )  v1
   )  v11
   WHERE ("rnk" < 11)
)  asceding
, (
   SELECT *
   FROM
     (
      SELECT
        "item_sk"
      , "rank"() OVER (ORDER BY "rank_col" DESC) "rnk"
      FROM
        (
         SELECT
           "ss_item_sk" "item_sk"
         , "avg"("ss_net_profit") "rank_col"
         FROM
           catalog.schema.store_sales ss1
         WHERE ("ss_store_sk" = 4)
         GROUP BY "ss_item_sk"
         HAVING ("avg"("ss_net_profit") > (DECIMAL '0.9' * (
                  SELECT "avg"("ss_net_profit") "rank_col"
                  FROM
                    catalog.schema.store_sales
                  WHERE ("ss_store_sk" = 4)
                     AND ("ss_addr_sk" IS NULL)
                  GROUP BY "ss_store_sk"
               )))
      )  v2
   )  v21
   WHERE ("rnk" < 11)
)  descending
, catalog.schema.item i1
, catalog.schema.item i2
WHERE ("asceding"."rnk" = "descending"."rnk")
   AND ("i1"."i_item_sk" = "asceding"."item_sk")
   AND ("i2"."i_item_sk" = "descending"."item_sk")
ORDER BY "asceding"."rnk" ASC,
   -- additional columns to assure results stability for larger scale factors; this is a deviation from TPC-DS specification
   "i1"."i_product_name" ASC, "i2"."i_product_name" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q34.sql
SELECT
  "c_last_name"
, "c_first_name"
, "c_salutation"
, "c_preferred_cust_flag"
, "ss_ticket_number"
, "cnt"
FROM
  (
   SELECT
     "ss_ticket_number"
   , "ss_customer_sk"
   , "count"(*) "cnt"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   , catalog.schema.household_demographics
   WHERE ("store_sales"."ss_sold_date_sk" = "date_dim"."d_date_sk")
      AND ("store_sales"."ss_store_sk" = "store"."s_store_sk")
      AND ("store_sales"."ss_hdemo_sk" = "household_demographics"."hd_demo_sk")
      AND (("date_dim"."d_dom" BETWEEN 1 AND 3)
         OR ("date_dim"."d_dom" BETWEEN 25 AND 28))
      AND (("household_demographics"."hd_buy_potential" = '>10000')
         OR ("household_demographics"."hd_buy_potential" = 'Unknown'))
      AND ("household_demographics"."hd_vehicle_count" > 0)
      AND ((CASE WHEN ("household_demographics"."hd_vehicle_count" > 0) THEN (CAST("household_demographics"."hd_dep_count" AS DECIMAL(7,2)) / "household_demographics"."hd_vehicle_count") ELSE null END) > DECIMAL '1.2')
      AND ("date_dim"."d_year" IN (1999   , (1999 + 1)   , (1999 + 2)))
      AND ("store"."s_county" IN ('Williamson County'   , 'Williamson County'   , 'Williamson County'   , 'Williamson County'   , 'Williamson County'   , 'Williamson County'   , 'Williamson County'   , 'Williamson County'))
   GROUP BY "ss_ticket_number", "ss_customer_sk"
)  dn
, catalog.schema.customer
WHERE ("ss_customer_sk" = "c_customer_sk")
   AND ("cnt" BETWEEN 15 AND 20)
ORDER BY "c_last_name" ASC, "c_first_name" ASC, "c_salutation" ASC, "c_preferred_cust_flag" DESC, "ss_ticket_number" ASC;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q89.sql
SELECT *
FROM
  (
   SELECT
     "i_category"
   , "i_class"
   , "i_brand"
   , "s_store_name"
   , "s_company_name"
   , "d_moy"
   , "sum"("ss_sales_price") "sum_sales"
   , "avg"("sum"("ss_sales_price")) OVER (PARTITION BY "i_category", "i_brand", "s_store_name", "s_company_name") "avg_monthly_sales"
   FROM
     catalog.schema.item
   , catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   WHERE ("ss_item_sk" = "i_item_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("ss_store_sk" = "s_store_sk")
      AND ("d_year" IN (1999))
      AND ((("i_category" IN ('Books'         , 'Electronics'         , 'Sports'))
            AND ("i_class" IN ('computers'         , 'stereo'         , 'football')))
         OR (("i_category" IN ('Men'         , 'Jewelry'         , 'Women'))
            AND ("i_class" IN ('shirts'         , 'birdal'         , 'dresses'))))
   GROUP BY "i_category", "i_class", "i_brand", "s_store_name", "s_company_name", "d_moy"
)  tmp1
WHERE ((CASE WHEN ("avg_monthly_sales" <> 0) THEN ("abs"(("sum_sales" - "avg_monthly_sales")) / "avg_monthly_sales") ELSE null END) > DECIMAL '0.1')
ORDER BY ("sum_sales" - "avg_monthly_sales") ASC, "s_store_name" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q92.sql
SELECT "sum"("ws_ext_discount_amt") "Excess Discount Amount"
FROM
  catalog.schema.web_sales
, catalog.schema.item
, catalog.schema.date_dim
WHERE ("i_manufact_id" = 350)
   AND ("i_item_sk" = "ws_item_sk")
   AND ("d_date" BETWEEN CAST('2000-01-27' AS DATE) AND (CAST('2000-01-27' AS DATE) + INTERVAL  '90' DAY))
   AND ("d_date_sk" = "ws_sold_date_sk")
   AND ("ws_ext_discount_amt" > (
      SELECT (DECIMAL '1.3' * "avg"("ws_ext_discount_amt"))
      FROM
        catalog.schema.web_sales
      , catalog.schema.date_dim
      WHERE ("ws_item_sk" = "i_item_sk")
         AND ("d_date" BETWEEN CAST('2000-01-27' AS DATE) AND (CAST('2000-01-27' AS DATE) + INTERVAL  '90' DAY))
         AND ("d_date_sk" = "ws_sold_date_sk")
   ))
ORDER BY "sum"("ws_ext_discount_amt") ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q32.sql
SELECT "sum"("cs_ext_discount_amt") "excess discount amount"
FROM
  catalog.schema.catalog_sales
, catalog.schema.item
, catalog.schema.date_dim
WHERE ("i_manufact_id" = 977)
   AND ("i_item_sk" = "cs_item_sk")
   AND ("d_date" BETWEEN CAST('2000-01-27' AS DATE) AND (CAST('2000-01-27' AS DATE) + INTERVAL  '90' DAY))
   AND ("d_date_sk" = "cs_sold_date_sk")
   AND ("cs_ext_discount_amt" > (
      SELECT (DECIMAL '1.3' * "avg"("cs_ext_discount_amt"))
      FROM
        catalog.schema.catalog_sales
      , catalog.schema.date_dim
      WHERE ("cs_item_sk" = "i_item_sk")
         AND ("d_date" BETWEEN CAST('2000-01-27' AS DATE) AND (CAST('2000-01-27' AS DATE) + INTERVAL  '90' DAY))
         AND ("d_date_sk" = "cs_sold_date_sk")
   ))
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q37.sql
SELECT
  "i_item_id"
, "i_item_desc"
, "i_current_price"
FROM
  catalog.schema.item
, catalog.schema.inventory
, catalog.schema.date_dim
, catalog.schema.catalog_sales
WHERE ("i_current_price" BETWEEN 68 AND (68 + 30))
   AND ("inv_item_sk" = "i_item_sk")
   AND ("d_date_sk" = "inv_date_sk")
   AND (CAST("d_date" AS DATE) BETWEEN CAST('2000-02-01' AS DATE) AND (CAST('2000-02-01' AS DATE) + INTERVAL  '60' DAY))
   AND ("i_manufact_id" IN (677, 940, 694, 808))
   AND ("inv_quantity_on_hand" BETWEEN 100 AND 500)
   AND ("cs_item_sk" = "i_item_sk")
GROUP BY "i_item_id", "i_item_desc", "i_current_price"
ORDER BY "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q82.sql
SELECT
  "i_item_id"
, "i_item_desc"
, "i_current_price"
FROM
  catalog.schema.item
, catalog.schema.inventory
, catalog.schema.date_dim
, catalog.schema.store_sales
WHERE ("i_current_price" BETWEEN 62 AND (62 + 30))
   AND ("inv_item_sk" = "i_item_sk")
   AND ("d_date_sk" = "inv_date_sk")
   AND (CAST("d_date" AS DATE) BETWEEN CAST('2000-05-25' AS DATE) AND (CAST('2000-05-25' AS DATE) + INTERVAL  '60' DAY))
   AND ("i_manufact_id" IN (129, 270, 821, 423))
   AND ("inv_quantity_on_hand" BETWEEN 100 AND 500)
   AND ("ss_item_sk" = "i_item_sk")
GROUP BY "i_item_id", "i_item_desc", "i_current_price"
ORDER BY "i_item_id" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q74.sql
WITH
  year_total AS (
   SELECT
     "c_customer_id" "customer_id"
   , "c_first_name" "customer_first_name"
   , "c_last_name" "customer_last_name"
   , "d_year" "YEAR"
   , "sum"("ss_net_paid") "year_total"
   , 's' "sale_type"
   FROM
     catalog.schema.customer
   , catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("c_customer_sk" = "ss_customer_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" IN (2001   , (2001 + 1)))
   GROUP BY "c_customer_id", "c_first_name", "c_last_name", "d_year"
UNION ALL    SELECT
     "c_customer_id" "customer_id"
   , "c_first_name" "customer_first_name"
   , "c_last_name" "customer_last_name"
   , "d_year" "YEAR"
   , "sum"("ws_net_paid") "year_total"
   , 'w' "sale_type"
   FROM
     catalog.schema.customer
   , catalog.schema.web_sales
   , catalog.schema.date_dim
   WHERE ("c_customer_sk" = "ws_bill_customer_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" IN (2001   , (2001 + 1)))
   GROUP BY "c_customer_id", "c_first_name", "c_last_name", "d_year"
) 
SELECT
  "t_s_secyear"."customer_id"
, "t_s_secyear"."customer_first_name"
, "t_s_secyear"."customer_last_name"
FROM
  year_total t_s_firstyear
, year_total t_s_secyear
, year_total t_w_firstyear
, year_total t_w_secyear
WHERE ("t_s_secyear"."customer_id" = "t_s_firstyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_w_secyear"."customer_id")
   AND ("t_s_firstyear"."customer_id" = "t_w_firstyear"."customer_id")
   AND ("t_s_firstyear"."sale_type" = 's')
   AND ("t_w_firstyear"."sale_type" = 'w')
   AND ("t_s_secyear"."sale_type" = 's')
   AND ("t_w_secyear"."sale_type" = 'w')
   AND ("t_s_firstyear"."year" = 2001)
   AND ("t_s_secyear"."year" = (2001 + 1))
   AND ("t_w_firstyear"."year" = 2001)
   AND ("t_w_secyear"."year" = (2001 + 1))
   AND ("t_s_firstyear"."year_total" > 0)
   AND ("t_w_firstyear"."year_total" > 0)
   AND ((CASE WHEN ("t_w_firstyear"."year_total" > 0) THEN ("t_w_secyear"."year_total" / "t_w_firstyear"."year_total") ELSE null END) > (CASE WHEN ("t_s_firstyear"."year_total" > 0) THEN ("t_s_secyear"."year_total" / "t_s_firstyear"."year_total") ELSE null END))
ORDER BY 1 ASC, 1 ASC, 1 ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q10.sql
SELECT
  "cd_gender"
, "cd_marital_status"
, "cd_education_status"
, "count"(*) "cnt1"
, "cd_purchase_estimate"
, "count"(*) "cnt2"
, "cd_credit_rating"
, "count"(*) "cnt3"
, "cd_dep_count"
, "count"(*) "cnt4"
, "cd_dep_employed_count"
, "count"(*) "cnt5"
, "cd_dep_college_count"
, "count"(*) "cnt6"
FROM
  catalog.schema.customer c
, catalog.schema.customer_address ca
, catalog.schema.customer_demographics
WHERE ("c"."c_current_addr_sk" = "ca"."ca_address_sk")
   AND ("ca_county" IN ('Rush County', 'Toole County', 'Jefferson County', 'Dona Ana County', 'La Porte County'))
   AND ("cd_demo_sk" = "c"."c_current_cdemo_sk")
   AND (EXISTS (
   SELECT *
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("c"."c_customer_sk" = "ss_customer_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2002)
      AND ("d_moy" BETWEEN 1 AND (1 + 3))
))
   AND ((EXISTS (
      SELECT *
      FROM
        catalog.schema.web_sales
      , catalog.schema.date_dim
      WHERE ("c"."c_customer_sk" = "ws_bill_customer_sk")
         AND ("ws_sold_date_sk" = "d_date_sk")
         AND ("d_year" = 2002)
         AND ("d_moy" BETWEEN 1 AND (1 + 3))
   ))
      OR (EXISTS (
      SELECT *
      FROM
        catalog.schema.catalog_sales
      , catalog.schema.date_dim
      WHERE ("c"."c_customer_sk" = "cs_ship_customer_sk")
         AND ("cs_sold_date_sk" = "d_date_sk")
         AND ("d_year" = 2002)
         AND ("d_moy" BETWEEN 1 AND (1 + 3))
   )))
GROUP BY "cd_gender", "cd_marital_status", "cd_education_status", "cd_purchase_estimate", "cd_credit_rating", "cd_dep_count", "cd_dep_employed_count", "cd_dep_college_count"
ORDER BY "cd_gender" ASC, "cd_marital_status" ASC, "cd_education_status" ASC, "cd_purchase_estimate" ASC, "cd_credit_rating" ASC, "cd_dep_count" ASC, "cd_dep_employed_count" ASC, "cd_dep_college_count" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q73.sql
SELECT
  "c_last_name"
, "c_first_name"
, "c_salutation"
, "c_preferred_cust_flag"
, "ss_ticket_number"
, "cnt"
FROM
  (
   SELECT
     "ss_ticket_number"
   , "ss_customer_sk"
   , "count"(*) "cnt"
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   , catalog.schema.store
   , catalog.schema.household_demographics
   WHERE ("store_sales"."ss_sold_date_sk" = "date_dim"."d_date_sk")
      AND ("store_sales"."ss_store_sk" = "store"."s_store_sk")
      AND ("store_sales"."ss_hdemo_sk" = "household_demographics"."hd_demo_sk")
      AND ("date_dim"."d_dom" BETWEEN 1 AND 2)
      AND (("household_demographics"."hd_buy_potential" = '>10000')
         OR ("household_demographics"."hd_buy_potential" = 'Unknown'))
      AND ("household_demographics"."hd_vehicle_count" > 0)
      AND ((CASE WHEN ("household_demographics"."hd_vehicle_count" > 0) THEN (CAST("household_demographics"."hd_dep_count" AS DECIMAL(7,2)) / "household_demographics"."hd_vehicle_count") ELSE null END) > 1)
      AND ("date_dim"."d_year" IN (1999   , (1999 + 1)   , (1999 + 2)))
      AND ("store"."s_county" IN ('Williamson County'   , 'Franklin Parish'   , 'Bronx County'   , 'Orange County'))
   GROUP BY "ss_ticket_number", "ss_customer_sk"
)  dj
, catalog.schema.customer
WHERE ("ss_customer_sk" = "c_customer_sk")
   AND ("cnt" BETWEEN 1 AND 5)
ORDER BY "cnt" DESC, "c_last_name" ASC,
   -- additional column to assure results stability for larger scale factors; this is a deviation from TPC-DS specification
   "ss_ticket_number" ASC;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q69.sql
SELECT
  "cd_gender"
, "cd_marital_status"
, "cd_education_status"
, "count"(*) "cnt1"
, "cd_purchase_estimate"
, "count"(*) "cnt2"
, "cd_credit_rating"
, "count"(*) "cnt3"
FROM
  catalog.schema.customer c
, catalog.schema.customer_address ca
, catalog.schema.customer_demographics
WHERE ("c"."c_current_addr_sk" = "ca"."ca_address_sk")
   AND ("ca_state" IN ('KY', 'GA', 'NM'))
   AND ("cd_demo_sk" = "c"."c_current_cdemo_sk")
   AND (EXISTS (
   SELECT *
   FROM
     catalog.schema.store_sales
   , catalog.schema.date_dim
   WHERE ("c"."c_customer_sk" = "ss_customer_sk")
      AND ("ss_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" BETWEEN 4 AND (4 + 2))
))
   AND (NOT (EXISTS (
   SELECT *
   FROM
     catalog.schema.web_sales
   , catalog.schema.date_dim
   WHERE ("c"."c_customer_sk" = "ws_bill_customer_sk")
      AND ("ws_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" BETWEEN 4 AND (4 + 2))
)))
   AND (NOT (EXISTS (
   SELECT *
   FROM
     catalog.schema.catalog_sales
   , catalog.schema.date_dim
   WHERE ("c"."c_customer_sk" = "cs_ship_customer_sk")
      AND ("cs_sold_date_sk" = "d_date_sk")
      AND ("d_year" = 2001)
      AND ("d_moy" BETWEEN 4 AND (4 + 2))
)))
GROUP BY "cd_gender", "cd_marital_status", "cd_education_status", "cd_purchase_estimate", "cd_credit_rating"
ORDER BY "cd_gender" ASC, "cd_marital_status" ASC, "cd_education_status" ASC, "cd_purchase_estimate" ASC, "cd_credit_rating" ASC
LIMIT 100;

-- testing/trino-benchmark-queries/src/main/resources/sql/trino/tpcds/q95.sql
WITH
  ws_wh AS (
   SELECT
     "ws1"."ws_order_number"
   , "ws1"."ws_warehouse_sk" "wh1"
   , "ws2"."ws_warehouse_sk" "wh2"
   FROM
     catalog.schema.web_sales ws1
   , catalog.schema.web_sales ws2
   WHERE ("ws1"."ws_order_number" = "ws2"."ws_order_number")
      AND ("ws1"."ws_warehouse_sk" <> "ws2"."ws_warehouse_sk")
) 
SELECT
  "count"(DISTINCT "ws_order_number") "order count"
, "sum"("ws_ext_ship_cost") "total shipping cost"
, "sum"("ws_net_profit") "total net profit"
FROM
  catalog.schema.web_sales ws1
, catalog.schema.date_dim
, catalog.schema.customer_address
, catalog.schema.web_site
WHERE (CAST("d_date" AS DATE) BETWEEN CAST('1999-2-01' AS DATE) AND (CAST('1999-2-01' AS DATE) + INTERVAL  '60' DAY))
   AND ("ws1"."ws_ship_date_sk" = "d_date_sk")
   AND ("ws1"."ws_ship_addr_sk" = "ca_address_sk")
   AND ("ca_state" = 'IL')
   AND ("ws1"."ws_web_site_sk" = "web_site_sk")
   AND ("web_company_name" = 'pri')
   AND ("ws1"."ws_order_number" IN (
   SELECT "ws_order_number"
   FROM
     ws_wh
))
   AND ("ws1"."ws_order_number" IN (
   SELECT "wr_order_number"
   FROM
     catalog.schema.web_returns
   , ws_wh
   WHERE ("wr_order_number" = "ws_wh"."ws_order_number")
))
ORDER BY "count"(DISTINCT "ws_order_number") ASC
LIMIT 100;
