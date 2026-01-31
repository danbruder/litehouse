module Page.Dashboard.Layout exposing
    ( viewHeader
    , viewSidebar
    , viewVersion
    , viewGitHubStatusBadge
    )

{-| Dashboard layout components: header, sidebar, and footer.
-}

import Html exposing (Html, a, aside, button, div, header, nav, p, span, text)
import Html.Attributes exposing (class, href)
import Html.Events exposing (onClick)
import Shared


-- VIEW FUNCTIONS


viewHeader : Shared.Model navigationKey -> Shared.GitHubStatus -> String -> msg -> msg -> Html msg
viewHeader shared githubStatus userName onShowCreateApp onLogout =
    header [ class "bg-litehouse-surface border-b border-litehouse-border px-6 py-4 flex justify-between items-center" ]
        [ div [ class "flex items-center gap-4" ]
            [ a [ href "/dashboard", class "text-xl font-semibold text-litehouse-text hover:opacity-80 transition-opacity" ] [ text "Litehouse" ]
            ]
        , div [ class "flex items-center gap-4" ]
            [ viewGitHubStatusBadge githubStatus
            , span [ class "text-sm text-litehouse-muted" ] [ text userName ]
            , button
                [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                , onClick onShowCreateApp
                ]
                [ text "+ New App" ]
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                , onClick onLogout
                ]
                [ text "Logout" ]
            ]
        ]


viewSidebar : Html msg
viewSidebar =
    aside [ class "w-56 bg-litehouse-surface border-r border-litehouse-border p-4" ]
        [ nav [ class "space-y-1" ]
            [ viewSidebarItem "My Apps" "/dashboard"
            , viewSidebarItemDisabled "Activity"
            , viewSidebarItemDisabled "Backups"
            , viewSidebarItem "Settings" "/settings"
            ]
        ]


viewSidebarItem : String -> String -> Html msg
viewSidebarItem label href_ =
    let
        baseClasses =
            "block w-full px-3 py-2 rounded-xl text-sm font-medium transition-colors text-left"

        classes =
            baseClasses ++ " text-litehouse-muted hover:bg-litehouse-bg hover:text-litehouse-text"
    in
    a [ class classes, href href_ ] [ text label ]


viewSidebarItemDisabled : String -> Html msg
viewSidebarItemDisabled label =
    let
        baseClasses =
            "block w-full px-3 py-2 rounded-xl text-sm font-medium transition-colors text-left"

        classes =
            baseClasses ++ " text-litehouse-muted cursor-not-allowed opacity-50"
    in
    button
        [ class classes
        , Html.Attributes.disabled True
        ]
        [ text label ]


viewGitHubStatusBadge : Shared.GitHubStatus -> Html msg
viewGitHubStatusBadge status =
    case status of
        Shared.GitHubConnected username ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-success/20 text-litehouse-success" ]
                [ text ("GitHub: " ++ username) ]

        Shared.GitHubNotConnected ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-warning/20 text-litehouse-warning" ]
                [ text "GitHub: Not connected" ]

        Shared.GitHubUnknown ->
            text ""


viewVersion : String -> Html msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "text-xs text-litehouse-muted" ] [ text ("v" ++ version) ]
