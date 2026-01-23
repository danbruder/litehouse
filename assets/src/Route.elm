module Route exposing
    ( Route(..)
    , fromUrl
    , toString
    , parser
    )

import Url
import Url.Parser as Parser exposing ((</>), Parser, oneOf, s, string)


-- TYPES


type Route
    = Login
    | Setup
    | Dashboard
    | AppDetail String


-- PARSER


parser : Parser (Route -> a) a
parser =
    oneOf
        [ Parser.map Login Parser.top
        , Parser.map Login (s "login")
        , Parser.map Setup (s "setup")
        , Parser.map Dashboard (s "dashboard")
        , Parser.map AppDetail (s "apps" </> string)
        ]


fromUrl : Url.Url -> Maybe Route
fromUrl url =
    Parser.parse parser url


toString : Route -> String
toString route =
    case route of
        Login ->
            "/login"

        Setup ->
            "/setup"

        Dashboard ->
            "/dashboard"

        AppDetail appName ->
            "/apps/" ++ appName
