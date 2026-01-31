module Layout exposing
    ( viewShell
    , viewHeader
    , viewSidebar
    , viewVersion
    , viewGitHubStatusBadge
    )

{-| Shared layout components for authenticated pages: header, sidebar, and footer.
-}

import Html exposing (Html, a, aside, button, div, footer, header, main_, nav, p, span, text)
import Html.Attributes exposing (class, href)
import Html.Events exposing (onClick)
import Route
import Shared


-- SHELL


{-| Render a complete page shell with header, sidebar, and footer wrapping the main content.
-}
viewShell :
    { route : Maybe Route.Route
    , shared : Shared.Model navigationKey
    , onLogout : msg
    , headerAction : Maybe { label : String, msg : msg }
    , content : Html msg
    }
    -> Html msg
viewShell config =
    let
        userName =
            case config.shared.user of
                Just user ->
                    user.fullName

                Nothing ->
                    ""
    in
    div [ class "min-h-screen bg-litehouse-bg flex flex-col" ]
        [ viewHeader config.shared config.shared.githubStatus userName config.headerAction config.onLogout
        , div [ class "flex flex-1" ]
            [ viewSidebar config.route
            , main_ [ class "flex-1 p-6" ]
                [ div [ class "max-w-6xl mx-auto" ]
                    [ config.content ]
                ]
            ]
        , footer [ class "p-4 text-center border-t border-litehouse-border" ]
            [ viewVersion config.shared.serverVersion
            ]
        ]


-- HEADER


{-| Render the page header with logo, GitHub status, user info, and action button.
-}
viewHeader : Shared.Model navigationKey -> Shared.GitHubStatus -> String -> Maybe { label : String, msg : msg } -> msg -> Html msg
viewHeader shared githubStatus userName headerAction onLogout =
    header [ class "bg-litehouse-surface border-b border-litehouse-border px-6 py-4 flex justify-between items-center" ]
        [ div [ class "flex items-center gap-4" ]
            [ a [ href "/dashboard", class "text-xl font-semibold text-litehouse-text hover:opacity-80 transition-opacity" ] [ text "Litehouse" ]
            ]
        , div [ class "flex items-center gap-4" ]
            [ viewGitHubStatusBadge githubStatus
            , span [ class "text-sm text-litehouse-muted" ] [ text userName ]
            , case headerAction of
                Just action ->
                    button
                        [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                        , onClick action.msg
                        ]
                        [ text action.label ]

                Nothing ->
                    text ""
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                , onClick onLogout
                ]
                [ text "Logout" ]
            ]
        ]


-- SIDEBAR


{-| Render the sidebar with navigation items. Highlights the active item based on current route.
-}
viewSidebar : Maybe Route.Route -> Html msg
viewSidebar maybeRoute =
    aside [ class "w-56 bg-litehouse-surface border-r border-litehouse-border p-4" ]
        [ nav [ class "space-y-1" ]
            [ viewSidebarItem "My Apps" "/dashboard" (isActive maybeRoute Route.Dashboard)
            , viewSidebarItemDisabled "Activity"
            , viewSidebarItemDisabled "Backups"
            , viewSidebarItem "Settings" "/settings" (isActive maybeRoute Route.Settings)
            ]
        ]


{-| Check if a route matches the current route for sidebar highlighting.
-}
isActive : Maybe Route.Route -> Route.Route -> Bool
isActive maybeRoute route =
    case ( maybeRoute, route ) of
        ( Just (Route.AppDetail _), Route.Dashboard ) ->
            -- App detail is part of Dashboard section
            True

        ( Just currentRoute, targetRoute ) ->
            currentRoute == targetRoute

        _ ->
            False


{-| Render an active or inactive sidebar item link.
-}
viewSidebarItem : String -> String -> Bool -> Html msg
viewSidebarItem label href_ isCurrentPage =
    let
        baseClasses =
            "block w-full px-3 py-2 rounded-xl text-sm font-medium transition-colors text-left"

        classes =
            if isCurrentPage then
                baseClasses ++ " bg-litehouse-bg text-litehouse-amber"

            else
                baseClasses ++ " text-litehouse-muted hover:bg-litehouse-bg hover:text-litehouse-text"
    in
    a [ class classes, href href_ ] [ text label ]


{-| Render a disabled sidebar item (for future features).
-}
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


-- GITHUB STATUS BADGE


{-| Render a badge showing GitHub connection status.
-}
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


-- VERSION


{-| Render the footer version text.
-}
viewVersion : String -> Html msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "text-xs text-litehouse-muted" ] [ text ("v" ++ version) ]
